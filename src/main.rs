use std::process::{Command, exit};
use std::env;
use std::time::Duration;
use std::thread::sleep;
use std::io::{self, Write, Read};
use std::os::unix::io::AsRawFd;
use std::fs;
use chrono::{Local, Timelike, Datelike};

// Crates pour daemon
use daemonize::Daemonize;
use std::fs::File;

// Crates pour xsct intégré
use std::f64::consts::E;
use std::os::raw::{c_double, c_int};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;
use clap::{Arg, ArgAction, Command as ClapCommand};

// Constants
const ESC_KEY: u8 = 27;
const COMMAND_XSCT: &str = "xsct";
const DAYS_PER_MONTH: f32 = 30.0; // Approximation for smoothing

// Constantes XSCT
const XSCT_VERSION: &str = "1.0";
const TEMPERATURE_NORM: i32 = 6500;
const TEMPERATURE_NIGHT: i32 = 4500;
const TEMPERATURE_ZERO: i32 = 700;
const GAMMA_MULT: f64 = 65535.0;
const GAMMA_K0GR: f64 = -1.47751309139817;
const GAMMA_K1GR: f64 = 0.28590164772055;
const GAMMA_K0BR: f64 = -4.38321650114872;
const GAMMA_K1BR: f64 = 0.6212158769447;
const GAMMA_K0RB: f64 = 1.75390204039018;
const GAMMA_K1RB: f64 = -0.1150805671482;
const GAMMA_K0GB: f64 = 1.49221604915144;
const GAMMA_K1GB: f64 = -0.07513509588921;
const BRIGHTHESS_DIV: f64 = 65470.988;
const DELTA_MIN: i32 = -1_000_000;

// Global state
#[derive(Debug)]
struct AppState {
    verbose: bool,
    location_name: String,
    daemon: bool,
}

// Sunrise/sunset times for the 15th of each month (in minutes since midnight - LOCAL TIME)
#[derive(Debug)]
struct MonthlyTimes {
    sunrise: [i32; 12],  // 0-11 for Jan-Dec (LOCAL TIME)
    sunset: [i32; 12],   // 0-11 for Jan-Dec (LOCAL TIME)
}

impl MonthlyTimes {
    fn new_for_timezone(timezone: &str) -> Self {
        // Adjust times slightly based on timezone longitude
        let longitude_offset = get_longitude_offset(timezone);
        
        MonthlyTimes {
            // January - adjusted for timezone
            sunrise: [
                8 * 60 + 40 + longitude_offset,    // 8:40
                7 * 60 + 57 + longitude_offset,    // 7:57 (February)
                6 * 60 + 57 + longitude_offset,    // 6:57 (March)
                6 * 60 + 49 + longitude_offset,    // 6:49 (April)
                5 * 60 + 54 + longitude_offset,    // 5:54 (May)
                5 * 60 + 29 + longitude_offset,    // 5:29 (June)
                5 * 60 + 47 + longitude_offset,    // 5:47 (July)
                6 * 60 + 31 + longitude_offset,    // 6:31 (August)
                7 * 60 + 10 + longitude_offset,    // 7:10 (September)
                8 * 60 + 6 + longitude_offset,     // 8:06 (October)
                7 * 60 + 59 + longitude_offset,    // 7:59 (November)
                8 * 60 + 39 + longitude_offset,    // 8:39 (December)
            ],
            sunset: [
                17 * 60 + 5 + longitude_offset,    // 17:05
                17 * 60 + 56 + longitude_offset,   // 17:56
                18 * 60 + 46 + longitude_offset,   // 18:46
                20 * 60 + 37 + longitude_offset,   // 20:37
                21 * 60 + 24 + longitude_offset,   // 21:24
                21 * 60 + 56 + longitude_offset,   // 21:56
                21 * 60 + 48 + longitude_offset,   // 21:48
                21 * 60 + 1 + longitude_offset,    // 21:01
                19 * 60 + 55 + longitude_offset,  // 19:55
                18 * 60 + 49 + longitude_offset,  // 18:49
                16 * 60 + 54 + longitude_offset,  // 16:54
                16 * 60 + 36 + longitude_offset,  // 16:36
            ],
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TempStatus {
    temp: i32,
    brightness: f64,
}

// XSCT functions
fn double_trim(x: f64, a: f64, b: f64) -> f64 {
    if x < a {
        a
    } else if x > b {
        b
    } else {
        x
    }
}

fn get_sct_for_screen(
    conn: &RustConnection,
    screen: usize,
    icrtc: i32,
    fdebug: bool,
) -> TempStatus {
    let root = conn.setup().roots[screen].root;
    let resources = conn
        .randr_get_screen_resources_current(root)
        .expect("Failed to get screen resources")
        .reply()
        .expect("Failed to get screen resources reply");

    let ncrtc = resources.crtcs.len();
    let mut n = ncrtc;
    let mut icrtc_start = icrtc;

    if icrtc >= 0 && (icrtc as usize) < ncrtc {
        n = 1;
    } else {
        icrtc_start = 0;
    }

    let mut gammar = 0.0;
    let mut gammag = 0.0;
    let mut gammab = 0.0;

    for c in (icrtc_start as usize)..(icrtc_start as usize + n) {
        let crtcxid = resources.crtcs[c];
        let gamma = conn
            .randr_get_crtc_gamma(crtcxid)
            .expect("Failed to get CRTC gamma")
            .reply()
            .expect("Failed to get CRTC gamma reply");

        let size = gamma.red.len();
        gammar += f64::from(gamma.red[size - 1]);
        gammag += f64::from(gamma.green[size - 1]);
        gammab += f64::from(gamma.blue[size - 1]);
    }

    let mut brightness = if gammar > gammag { gammar } else { gammag };
    brightness = if gammab > brightness {
        gammab
    } else {
        brightness
    };

    let mut temp = 0.0;

    if brightness > 0.0 && n > 0 {
        gammar /= brightness;
        gammag /= brightness;
        gammab /= brightness;
        brightness /= n as f64;
        brightness /= BRIGHTHESS_DIV;
        brightness = double_trim(brightness, 0.0, 1.0);

        if fdebug {
            eprintln!(
                "DEBUG: Gamma: {}, {}, {}, brightness: {}",
                gammar, gammag, gammab, brightness
            );
        }

        let gammad = gammab - gammar;
        if gammad < 0.0 {
            if gammab > 0.0 {
                temp = ((gammag + 1.0 + gammad - (GAMMA_K0GR + GAMMA_K0BR))
                    / (GAMMA_K1GR + GAMMA_K1BR))
                    .exp()
                    + TEMPERATURE_ZERO as f64;
            } else {
                temp = if gammag > 0.0 {
                    ((gammag - GAMMA_K0GR) / GAMMA_K1GR).exp() + TEMPERATURE_ZERO as f64
                } else {
                    TEMPERATURE_ZERO as f64
                };
            }
        } else {
            temp = ((gammag + 1.0 - gammad - (GAMMA_K0GB + GAMMA_K0RB))
                / (GAMMA_K1GB + GAMMA_K1RB))
                .exp()
                + (TEMPERATURE_NORM - TEMPERATURE_ZERO) as f64;
        }
    } else {
        brightness = double_trim(brightness, 0.0, 1.0);
    }

    TempStatus {
        temp: (temp + 0.5) as i32,
        brightness,
    }
}

fn sct_for_screen(
    conn: &RustConnection,
    screen: usize,
    icrtc: i32,
    temp_status: TempStatus,
    fdebug: bool,
) {
    let root = conn.setup().roots[screen].root;
    let resources = conn
        .randr_get_screen_resources_current(root)
        .expect("Failed to get screen resources")
        .reply()
        .expect("Failed to get screen resources reply");

    let t = temp_status.temp as f64;
    let b = double_trim(temp_status.brightness, 0.0, 1.0);

    let (gammar, gammag, gammab) = if temp_status.temp < TEMPERATURE_NORM {
        let gammar = 1.0;
        let (gammag, gammab) = if temp_status.temp > TEMPERATURE_ZERO {
            let g = (t - TEMPERATURE_ZERO as f64).ln();
            (
                double_trim(GAMMA_K0GR + GAMMA_K1GR * g, 0.0, 1.0),
                double_trim(GAMMA_K0BR + GAMMA_K1BR * g, 0.0, 1.0),
            )
        } else {
            (0.0, 0.0)
        };
        (gammar, gammag, gammab)
    } else {
        let g = (t - (TEMPERATURE_NORM - TEMPERATURE_ZERO) as f64).ln();
        (
            double_trim(GAMMA_K0RB + GAMMA_K1RB * g, 0.0, 1.0),
            double_trim(GAMMA_K0GB + GAMMA_K1GB * g, 0.0, 1.0),
            1.0,
        )
    };

    if fdebug {
        eprintln!(
            "DEBUG: Gamma: {}, {}, {}, brightness: {}",
            gammar, gammag, gammab, b
        );
    }

    let ncrtc = resources.crtcs.len();
    let mut n = ncrtc;
    let mut icrtc_start = icrtc;

    if icrtc >= 0 && (icrtc as usize) < ncrtc {
        n = 1;
    } else {
        icrtc_start = 0;
    }

    for c in (icrtc_start as usize)..(icrtc_start as usize + n) {
        let crtcxid = resources.crtcs[c];
        let size_reply = conn
            .randr_get_crtc_gamma_size(crtcxid)
            .expect("Failed to get CRTC gamma size")
            .reply()
            .expect("Failed to get CRTC gamma size reply");
        let size = size_reply.size as usize;

        let mut red = vec![0u16; size];
        let mut green = vec![0u16; size];
        let mut blue = vec![0u16; size];

        for i in 0..size {
            let g = GAMMA_MULT * b * (i as f64) / (size as f64);
            red[i] = (g * gammar + 0.5) as u16;
            green[i] = (g * gammag + 0.5) as u16;
            blue[i] = (g * gammab + 0.5) as u16;
        }

        conn.randr_set_crtc_gamma(crtcxid, &red, &green, &blue)
            .expect("Failed to set CRTC gamma");
    }
}

fn bound_temp(temp: &mut TempStatus) {
    if temp.temp <= 0 {
        eprintln!("WARNING! Temperatures below 0 cannot be displayed.");
        temp.temp = TEMPERATURE_NORM;
    } else if temp.temp < TEMPERATURE_ZERO {
        eprintln!(
            "WARNING! Temperatures below {} cannot be displayed.",
            TEMPERATURE_ZERO
        );
        temp.temp = TEMPERATURE_ZERO;
    }

    if temp.brightness < 0.0 {
        eprintln!("WARNING! Brightness values below 0.0 cannot be displayed.");
        temp.brightness = 0.0;
    } else if temp.brightness > 1.0 {
        eprintln!("WARNING! Brightness values above 1.0 cannot be displayed.");
        temp.brightness = 1.0;
    }
}

fn xsct_set_temperature(kelvin: i32) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, _) = RustConnection::connect(None)?;
    let screens = conn.setup().roots.len();
    
    let temp = TempStatus {
        temp: if kelvin == 0 { TEMPERATURE_NORM } else { kelvin },
        brightness: 1.0,
    };
    
    for screen in 0..screens {
        sct_for_screen(&conn, screen, -1, temp, false);
    }
    
    Ok(())
}

// Get approximate longitude offset for timezone (in minutes)
fn get_longitude_offset(timezone: &str) -> i32 {
    // Extended timezone database with major cities worldwide
    match timezone {
        // Europe (UTC-1 to UTC+3)
        "Atlantic/Azores" => -60,   // Portugal (Azores)
        "Atlantic/Madeira" => -30,  // Portugal (Madeira)
        "Europe/Lisbon" | "Atlantic/Canary" => -30, // Portugal, Canary Islands
        "Europe/London" | "Europe/Dublin" | "Europe/Guernsey" | "Europe/Isle_of_Man" | "Europe/Jersey" => -30,
        "Africa/Casablanca" | "Africa/El_Aaiun" => -30, // Morocco, Western Sahara
        
        // Western Europe (UTC+0/+1 depending on DST)
        "Europe/Paris" | "Europe/Brussels" | "Europe/Amsterdam" | "Europe/Luxembourg" => 0,
        "Europe/Monaco" | "Europe/Andorra" | "Europe/Madrid" => 0,
        "Europe/Gibraltar" | "Africa/Algiers" | "Africa/Tunis" => 0,
        
        // Central Europe (UTC+1/+2)
        "Europe/Berlin" | "Europe/Vienna" | "Europe/Zurich" | "Europe/Rome" => 15,
        "Europe/Vatican" | "Europe/San_Marino" | "Europe/Malta" => 15,
        "Europe/Prague" | "Europe/Warsaw" | "Europe/Budapest" | "Europe/Bratislava" => 15,
        "Europe/Belgrade" | "Europe/Sarajevo" | "Europe/Skopje" | "Europe/Zagreb" => 15,
        "Europe/Tirane" | "Europe/Sofia" | "Europe/Bucharest" => 15,
        "Africa/Cairo" => 15, // Egypt
        
        // Eastern Europe (UTC+2/+3)
        "Europe/Helsinki" | "Europe/Tallinn" | "Europe/Riga" | "Europe/Vilnius" => 30,
        "Europe/Kiev" | "Europe/Chisinau" | "Europe/Uzhgorod" | "Europe/Zaporozhye" => 30,
        "Europe/Istanbul" | "Europe/Athens" | "Europe/Nicosia" => 30,
        "Asia/Beirut" | "Asia/Damascus" | "Asia/Amman" | "Asia/Jerusalem" => 30,
        "Asia/Gaza" | "Asia/Hebron" => 30,
        "Africa/Johannesburg" | "Africa/Windhoek" => 30, // South Africa, Namibia
        
        // Further east Europe/Russia (UTC+3)
        "Europe/Moscow" | "Europe/Simferopol" | "Europe/Kirov" | "Europe/Volgograd" => 45,
        "Europe/Astrakhan" | "Europe/Saratov" | "Europe/Ulyanovsk" => 45,
        "Europe/Samara" => 60,
        "Asia/Yerevan" | "Asia/Tbilisi" | "Asia/Baku" => 45,
        
        // Middle East (UTC+3 to UTC+4:30)
        "Asia/Riyadh" | "Asia/Qatar" | "Asia/Bahrain" | "Asia/Kuwait" => 45,
        "Asia/Aden" | "Asia/Muscat" => 45,
        "Asia/Dubai" => 60,
        "Asia/Tehran" => 75, // UTC+3:30
        "Asia/Kabul" => 105, // UTC+4:30
        
        // South Asia (UTC+5 to UTC+5:30)
        "Asia/Karachi" | "Asia/Tashkent" => 120,
        "Asia/Yekaterinburg" => 120,
        "Asia/Colombo" => 135, // UTC+5:30
        "Asia/Kolkata" | "Asia/Calcutta" => 135, // UTC+5:30
        "Asia/Kathmandu" => 142, // UTC+5:45
        
        // Southeast Asia (UTC+6 to UTC+7)
        "Asia/Dhaka" | "Asia/Almaty" => 150,
        "Asia/Novosibirsk" => 150,
        "Asia/Yangon" => 157, // UTC+6:30
        "Asia/Bangkok" | "Asia/Ho_Chi_Minh" | "Asia/Phnom_Penh" | "Asia/Vientiane" => 165,
        "Asia/Jakarta" | "Asia/Pontianak" => 165,
        "Asia/Krasnoyarsk" => 165,
        
        // East Asia (UTC+7 to UTC+9)
        "Asia/Shanghai" | "Asia/Beijing" | "Asia/Hong_Kong" | "Asia/Macau" => 180,
        "Asia/Taipei" | "Asia/Ulaanbaatar" => 180,
        "Asia/Singapore" | "Asia/Kuala_Lumpur" => 180,
        "Asia/Manila" | "Asia/Makassar" => 180,
        "Asia/Irkutsk" => 180,
        "Asia/Seoul" | "Asia/Tokyo" => 195,
        "Asia/Yakutsk" => 195,
        
        // Australia/Oceania (UTC+8 to UTC+12)
        "Australia/Perth" => 180,
        "Australia/Eucla" => 187, // UTC+8:45
        "Asia/Jayapura" => 195,
        "Australia/Darwin" => 195,
        "Australia/Adelaide" => 195,
        "Australia/Brisbane" | "Australia/Lindeman" => 195,
        "Australia/Sydney" | "Australia/Melbourne" | "Australia/Hobart" => 195,
        "Australia/Lord_Howe" => 202, // UTC+10:30
        "Pacific/Guadalcanal" | "Pacific/Noumea" => 210,
        "Pacific/Norfolk" => 210,
        "Pacific/Fiji" | "Pacific/Tarawa" => 240,
        "Pacific/Auckland" | "Pacific/Majuro" => 255,
        "Pacific/Chatham" => 268, // UTC+12:45
        "Pacific/Apia" | "Pacific/Fakaofo" => 255,
        
        // North America - Pacific (UTC-8 to UTC-7)
        "America/Los_Angeles" | "America/Vancouver" | "America/Tijuana" => -480,
        "America/Whitehorse" | "America/Dawson" => -480,
        "America/Phoenix" | "America/Hermosillo" => -420, // No DST
        "America/Denver" | "America/Edmonton" | "America/Boise" => -420,
        "America/Ciudad_Juarez" | "America/Ojinaga" => -420,
        
        // North America - Central (UTC-6)
        "America/Chicago" | "America/Winnipeg" | "America/Rainy_River" => -360,
        "America/Matamoros" | "America/Mexico_City" | "America/Monterrey" => -360,
        "America/Regina" | "America/Swift_Current" => -360, // No DST
        
        // North America - Eastern (UTC-5)
        "America/New_York" | "America/Toronto" | "America/Montreal" => -300,
        "America/Detroit" | "America/Indiana/Indianapolis" => -300,
        "America/Cancun" | "America/Havana" | "America/Port-au-Prince" => -300,
        "America/Nassau" | "America/Jamaica" => -300,
        "America/Panama" | "America/Bogota" | "America/Lima" => -300,
        
        // South America (UTC-5 to UTC-3)
        "America/Caracas" => -270, // UTC-4:30
        "America/Santiago" | "America/Asuncion" => -240,
        "America/La_Paz" | "America/Guyana" => -240,
        "America/Argentina/Buenos_Aires" | "America/Montevideo" => -180,
        "America/Sao_Paulo" | "America/Fortaleza" => -180,
        "America/Nuuk" | "America/Miquelon" => -180,
        "America/Godthab" => -180,
        "America/St_Johns" => -210, // UTC-3:30
        
        // Africa (Various)
        "America/Noronha" => -120, // UTC-2
        "Atlantic/South_Georgia" => -120,
        "Atlantic/Cape_Verde" => -60,
        "Africa/Abidjan" | "Africa/Accra" | "Africa/Bamako" => -30,
        "Africa/Algiers" | "Africa/Tunis" | "Africa/Tripoli" => 0,
        "Africa/Windhoek" => 30,
        
        // Pacific Islands
        "Pacific/Honolulu" => -600,
        "Pacific/Marquesas" => -570, // UTC-9:30
        "Pacific/Gambier" => -540,
        "Pacific/Pitcairn" => -480,
        "Pacific/Easter" => -360,
        "Pacific/Galapagos" => -360,
        "Pacific/Tahiti" => -600,
        
        // Default to Central Europe
        _ => 0,
    }
}

// Try to guess location from timezone
fn guess_location_from_system() -> Option<(String, String)> {
    // Try to read /etc/timezone first
    if let Ok(content) = fs::read_to_string("/etc/timezone") {
        let tz = content.trim();
        if let Some(name) = timezone_to_location_name(tz) {
            return Some((tz.to_string(), name));
        }
    }
    
    // Try to read symbolic link /etc/localtime
    if let Ok(target) = fs::read_link("/etc/localtime") {
        if let Some(tz_str) = target.to_str() {
            // Extract timezone from path like "/usr/share/zoneinfo/Europe/Brussels"
            if let Some(tz) = tz_str.strip_prefix("/usr/share/zoneinfo/") {
                if let Some(name) = timezone_to_location_name(tz) {
                    return Some((tz.to_string(), name));
                }
            }
        }
    }
    
    None
}

// Extended database mapping timezones to location names
fn timezone_to_location_name(timezone: &str) -> Option<String> {
    let name = match timezone {
        // Europe
        "Europe/Paris" => "Paris, France",
        "Europe/Brussels" => "Brussels, Belgium",
        "Europe/London" => "London, United Kingdom",
        "Europe/Berlin" => "Berlin, Germany",
        "Europe/Madrid" => "Madrid, Spain",
        "Europe/Rome" => "Rome, Italy",
        "Europe/Amsterdam" => "Amsterdam, Netherlands",
        "Europe/Lisbon" => "Lisbon, Portugal",
        "Europe/Vienna" => "Vienna, Austria",
        "Europe/Zurich" => "Zurich, Switzerland",
        "Europe/Warsaw" => "Warsaw, Poland",
        "Europe/Prague" => "Prague, Czech Republic",
        "Europe/Stockholm" => "Stockholm, Sweden",
        "Europe/Oslo" => "Oslo, Norway",
        "Europe/Copenhagen" => "Copenhagen, Denmark",
        "Europe/Helsinki" => "Helsinki, Finland",
        "Europe/Moscow" => "Moscow, Russia",
        "Europe/Kiev" => "Kyiv, Ukraine",
        "Europe/Bucharest" => "Bucharest, Romania",
        "Europe/Budapest" => "Budapest, Hungary",
        "Europe/Athens" => "Athens, Greece",
        "Europe/Dublin" => "Dublin, Ireland",
        "Europe/Sofia" => "Sofia, Bulgaria",
        "Europe/Belgrade" => "Belgrade, Serbia",
        "Europe/Zagreb" => "Zagreb, Croatia",
        "Europe/Sarajevo" => "Sarajevo, Bosnia and Herzegovina",
        "Europe/Skopje" => "Skopje, North Macedonia",
        "Europe/Tirane" => "Tirana, Albania",
        "Europe/Minsk" => "Minsk, Belarus",
        "Europe/Riga" => "Riga, Latvia",
        "Europe/Vilnius" => "Vilnius, Lithuania",
        "Europe/Tallinn" => "Tallinn, Estonia",
        "Europe/Chisinau" => "Chisinau, Moldova",
        "Europe/Bratislava" => "Bratislava, Slovakia",
        "Europe/Ljubljana" => "Ljubljana, Slovenia",
        "Europe/Luxembourg" => "Luxembourg City, Luxembourg",
        "Europe/Valletta" => "Valletta, Malta",
        "Europe/Monaco" => "Monaco",
        "Europe/San_Marino" => "San Marino",
        "Europe/Vatican" => "Vatican City",
        "Europe/Andorra" => "Andorra la Vella, Andorra",
        "Europe/Istanbul" => "Istanbul, Turkey",
        "Europe/Nicosia" => "Nicosia, Cyprus",
        
        // North America
        "America/New_York" | "US/Eastern" => "New York City, USA",
        "America/Chicago" | "US/Central" => "Chicago, USA",
        "America/Denver" | "US/Mountain" => "Denver, USA",
        "America/Los_Angeles" | "US/Pacific" => "Los Angeles, USA",
        "America/Phoenix" => "Phoenix, USA",
        "America/Anchorage" => "Anchorage, USA",
        "America/Honolulu" => "Honolulu, USA",
        "America/Toronto" => "Toronto, Canada",
        "America/Vancouver" => "Vancouver, Canada",
        "America/Montreal" => "Montreal, Canada",
        "America/Winnipeg" => "Winnipeg, Canada",
        "America/Edmonton" => "Edmonton, Canada",
        "America/Mexico_City" => "Mexico City, Mexico",
        "America/Cancun" => "Cancun, Mexico",
        "America/Havana" => "Havana, Cuba",
        "America/Port-au-Prince" => "Port-au-Prince, Haiti",
        "America/Santo_Domingo" => "Santo Domingo, Dominican Republic",
        "America/San_Juan" => "San Juan, Puerto Rico",
        "America/Nassau" => "Nassau, Bahamas",
        "America/Jamaica" => "Kingston, Jamaica",
        "America/Managua" => "Managua, Nicaragua",
        "America/Panama" => "Panama City, Panama",
        "America/Bogota" => "Bogota, Colombia",
        "America/Lima" => "Lima, Peru",
        "America/Caracas" => "Caracas, Venezuela",
        "America/Georgetown" => "Georgetown, Guyana",
        "America/Paramaribo" => "Paramaribo, Suriname",
        
        // South America
        "America/Santiago" => "Santiago, Chile",
        "America/Buenos_Aires" => "Buenos Aires, Argentina",
        "America/Sao_Paulo" => "Sao Paulo, Brazil",
        "America/Rio_de_Janeiro" => "Rio de Janeiro, Brazil",
        "America/Fortaleza" => "Fortaleza, Brazil",
        "America/Asuncion" => "Asuncion, Paraguay",
        "America/Montevideo" => "Montevideo, Uruguay",
        "America/La_Paz" => "La Paz, Bolivia",
        "America/Guayaquil" => "Guayaquil, Ecuador",
        "America/Quito" => "Quito, Ecuador",
        "America/Cayenne" => "Cayenne, French Guiana",
        
        // Asia
        "Asia/Tokyo" => "Tokyo, Japan",
        "Asia/Shanghai" => "Shanghai, China",
        "Asia/Beijing" => "Beijing, China",
        "Asia/Hong_Kong" => "Hong Kong",
        "Asia/Macau" => "Macau",
        "Asia/Taipei" => "Taipei, Taiwan",
        "Asia/Seoul" => "Seoul, South Korea",
        "Asia/Pyongyang" => "Pyongyang, North Korea",
        "Asia/Ulaanbaatar" => "Ulaanbaatar, Mongolia",
        "Asia/Singapore" => "Singapore",
        "Asia/Kuala_Lumpur" => "Kuala Lumpur, Malaysia",
        "Asia/Jakarta" => "Jakarta, Indonesia",
        "Asia/Bangkok" => "Bangkok, Thailand",
        "Asia/Manila" => "Manila, Philippines",
        "Asia/Ho_Chi_Minh" => "Ho Chi Minh City, Vietnam",
        "Asia/Hanoi" => "Hanoi, Vietnam",
        "Asia/Phnom_Penh" => "Phnom Penh, Cambodia",
        "Asia/Vientiane" => "Vientiane, Laos",
        "Asia/Yangon" => "Yangon, Myanmar",
        "Asia/Dhaka" => "Dhaka, Bangladesh",
        "Asia/Kolkata" => "Kolkata, India",
        "Asia/Delhi" => "New Delhi, India",
        "Asia/Mumbai" => "Mumbai, India",
        "Asia/Chennai" => "Chennai, India",
        "Asia/Karachi" => "Karachi, Pakistan",
        "Asia/Lahore" => "Lahore, Pakistan",
        "Asia/Kabul" => "Kabul, Afghanistan",
        "Asia/Tehran" => "Tehran, Iran",
        "Asia/Baghdad" => "Baghdad, Iraq",
        "Asia/Riyadh" => "Riyadh, Saudi Arabia",
        "Asia/Dubai" => "Dubai, UAE",
        "Asia/Muscat" => "Muscat, Oman",
        "Asia/Doha" => "Doha, Qatar",
        "Asia/Kuwait" => "Kuwait City, Kuwait",
        "Asia/Bahrain" => "Manama, Bahrain",
        "Asia/Amman" => "Amman, Jordan",
        "Asia/Beirut" => "Beirut, Lebanon",
        "Asia/Damascus" => "Damascus, Syria",
        "Asia/Jerusalem" => "Jerusalem, Israel",
        "Asia/Gaza" | "Asia/Hebron" => "Palestine",
        "Asia/Yerevan" => "Yerevan, Armenia",
        "Asia/Baku" => "Baku, Azerbaijan",
        "Asia/Tbilisi" => "Tbilisi, Georgia",
        "Asia/Ashgabat" => "Ashgabat, Turkmenistan",
        "Asia/Tashkent" => "Tashkent, Uzbekistan",
        "Asia/Dushanbe" => "Dushanbe, Tajikistan",
        "Asia/Bishkek" => "Bishkek, Kyrgyzstan",
        "Asia/Almaty" => "Almaty, Kazakhstan",
        "Asia/Colombo" => "Colombo, Sri Lanka",
        "Asia/Kathmandu" => "Kathmandu, Nepal",
        "Asia/Thimphu" => "Thimphu, Bhutan",
        "Asia/Male" => "Male, Maldives",
        
        // Africa
        "Africa/Cairo" => "Cairo, Egypt",
        "Africa/Johannesburg" => "Johannesburg, South Africa",
        "Africa/Cape_Town" => "Cape Town, South Africa",
        "Africa/Lagos" => "Lagos, Nigeria",
        "Africa/Kinshasa" => "Kinshasa, DR Congo",
        "Africa/Nairobi" => "Nairobi, Kenya",
        "Africa/Addis_Ababa" => "Addis Ababa, Ethiopia",
        "Africa/Dar_es_Salaam" => "Dar es Salaam, Tanzania",
        "Africa/Khartoum" => "Khartoum, Sudan",
        "Africa/Algiers" => "Algiers, Algeria",
        "Africa/Casablanca" => "Casablanca, Morocco",
        "Africa/Tunis" => "Tunis, Tunisia",
        "Africa/Tripoli" => "Tripoli, Libya",
        "Africa/Accra" => "Accra, Ghana",
        "Africa/Dakar" => "Dakar, Senegal",
        "Africa/Abidjan" => "Abidjan, Ivory Coast",
        "Africa/Bamako" => "Bamako, Mali",
        "Africa/Ouagadougou" => "Ouagadougou, Burkina Faso",
        "Africa/Conakry" => "Conakry, Guinea",
        "Africa/Freetown" => "Freetown, Sierra Leone",
        "Africa/Monrovia" => "Monrovia, Liberia",
        "Africa/Lome" => "Lome, Togo",
        "Africa/Porto-Novo" => "Porto-Novo, Benin",
        "Africa/Niamey" => "Niamey, Niger",
        "Africa/Ndjamena" => "Ndjamena, Chad",
        "Africa/Bangui" => "Bangui, Central African Republic",
        "Africa/Brazzaville" => "Brazzaville, Republic of the Congo",
        "Africa/Luanda" => "Luanda, Angola",
        "Africa/Lusaka" => "Lusaka, Zambia",
        "Africa/Harare" => "Harare, Zimbabwe",
        "Africa/Maputo" => "Maputo, Mozambique",
        "Africa/Blantyre" => "Blantyre, Malawi",
        "Africa/Gaborone" => "Gaborone, Botswana",
        "Africa/Maseru" => "Maseru, Lesotho",
        "Africa/Mbabane" => "Mbabane, Eswatini",
        "Africa/Mogadishu" => "Mogadishu, Somalia",
        "Africa/Djibouti" => "Djibouti City, Djibouti",
        "Africa/Asmara" => "Asmara, Eritrea",
        "Africa/Bujumbura" => "Bujumbura, Burundi",
        "Africa/Kigali" => "Kigali, Rwanda",
        "Africa/Kampala" => "Kampala, Uganda",
        "Africa/Douala" => "Douala, Cameroon",
        "Africa/Libreville" => "Libreville, Gabon",
        "Africa/Malabo" => "Malabo, Equatorial Guinea",
        "Africa/Sao_Tome" => "Sao Tome, Sao Tome and Principe",
        "Africa/Windhoek" => "Windhoek, Namibia",
        "Africa/Port_Louis" => "Port Louis, Mauritius",
        "Africa/Victoria" => "Victoria, Seychelles",
        "Africa/Nouakchott" => "Nouakchott, Mauritania",
        "Africa/Banjul" => "Banjul, Gambia",
        "Africa/Guinea-Bissau" => "Bissau, Guinea-Bissau",
        
        // Australia/Oceania
        "Australia/Sydney" => "Sydney, Australia",
        "Australia/Melbourne" => "Melbourne, Australia",
        "Australia/Brisbane" => "Brisbane, Australia",
        "Australia/Perth" => "Perth, Australia",
        "Australia/Adelaide" => "Adelaide, Australia",
        "Australia/Hobart" => "Hobart, Australia",
        "Australia/Darwin" => "Darwin, Australia",
        "Australia/Canberra" => "Canberra, Australia",
        "Pacific/Auckland" => "Auckland, New Zealand",
        "Pacific/Wellington" => "Wellington, New Zealand",
        "Pacific/Fiji" => "Suva, Fiji",
        "Pacific/Port_Moresby" => "Port Moresby, Papua New Guinea",
        "Pacific/Guadalcanal" => "Honiara, Solomon Islands",
        "Pacific/Noumea" => "Noumea, New Caledonia",
        "Pacific/Tarawa" => "Tarawa, Kiribati",
        "Pacific/Majuro" => "Majuro, Marshall Islands",
        "Pacific/Palau" => "Ngerulmud, Palau",
        "Pacific/Chuuk" => "Chuuk, Micronesia",
        "Pacific/Guam" => "Hagatna, Guam",
        "Pacific/Saipan" => "Saipan, Northern Mariana Islands",
        "Pacific/Honolulu" => "Honolulu, Hawaii, USA",
        "Pacific/Tahiti" => "Papeete, French Polynesia",
        "Pacific/Rarotonga" => "Avarua, Cook Islands",
        "Pacific/Apia" => "Apia, Samoa",
        "Pacific/Niue" => "Alofi, Niue",
        "Pacific/Tongatapu" => "Nuku'alofa, Tonga",
        "Pacific/Funafuti" => "Funafuti, Tuvalu",
        "Pacific/Wake" => "Wake Island, USA",
        "Pacific/Easter" => "Easter Island, Chile",
        
        // Antarctica (for completeness)
        "Antarctica/McMurdo" => "McMurdo Station, Antarctica",
        "Antarctica/Casey" => "Casey Station, Antarctica",
        "Antarctica/Davis" => "Davis Station, Antarctica",
        "Antarctica/Mawson" => "Mawson Station, Antarctica",
        "Antarctica/Palmer" => "Palmer Station, Antarctica",
        "Antarctica/Rothera" => "Rothera Station, Antarctica",
        "Antarctica/Syowa" => "Syowa Station, Antarctica",
        "Antarctica/Troll" => "Troll Station, Antarctica",
        "Antarctica/Vostok" => "Vostok Station, Antarctica",
        
        // Generic fallbacks for regions
        tz if tz.starts_with("Europe/") => "Europe",
        tz if tz.starts_with("America/") => "Americas",
        tz if tz.starts_with("Asia/") => "Asia",
        tz if tz.starts_with("Africa/") => "Africa",
        tz if tz.starts_with("Australia/") => "Australia",
        tz if tz.starts_with("Pacific/") => "Pacific Islands",
        tz if tz.starts_with("Atlantic/") => "Atlantic Region",
        tz if tz.starts_with("Indian/") => "Indian Ocean Region",
        tz if tz.starts_with("Antarctica/") => "Antarctica",
        
        // Final fallback
        _ => return None,
    };
    
    Some(name.to_string())
}

// Get current LOCAL time in minutes since midnight
fn get_current_local_time() -> i32 {
    let now = Local::now();
    (now.hour() as i32) * 60 + (now.minute() as i32)
}

// Get current month (1-12) and day (1-31)
fn get_current_month_day() -> (usize, i32) {
    let now = Local::now();
    (now.month() as usize, now.day() as i32)
}

// Get current minute (0-59)
fn get_current_minute() -> u32 {
    Local::now().minute()
}

// Get smoothed sunrise/sunset times (using your original algorithm)
fn get_smoothed_day_times(monthly_times: &MonthlyTimes, month: usize, day: i32) -> (i32, i32) {
    // Month is 1-12, convert to 0-11 for array indexing
    let month_index = month - 1;
    
    let (month1, month2, day_in_month) = if day <= 15 {
        // First half of month
        let month1 = if month_index == 0 { 11 } else { month_index - 1 };
        let month2 = month_index;
        let day_in_month = day + 15;
        (month1, month2, day_in_month)
    } else {
        // Second half of month
        let month1 = month_index;
        let month2 = (month_index + 1) % 12;
        let day_in_month = day - 15;
        (month1, month2, day_in_month)
    };
    
    // Calculate interpolation ratio
    let ratio = day_in_month as f32 / DAYS_PER_MONTH;
    
    // Linear interpolation
    let sunrise = (monthly_times.sunrise[month1] as f32 +
                  (monthly_times.sunrise[month2] as f32 - monthly_times.sunrise[month1] as f32) * ratio)
                  .round() as i32;
    
    let sunset = (monthly_times.sunset[month1] as f32 +
                  (monthly_times.sunset[month2] as f32 - monthly_times.sunset[month1] as f32) * ratio)
                  .round() as i32;
    
    (sunrise, sunset)
}

// Format number with leading zero
fn format_number(value: i32, format: &str) -> String {
    if format == "00" && value < 10 {
        format!("0{}", value)
    } else {
        value.to_string()
    }
}

// Format time from minutes since midnight
fn format_time(minutes: i32) -> String {
    format!("{}:{}",
        format_number(minutes / 60, "00"),
        format_number(minutes % 60, "00"))
}

// Manage brightness cycle - CALLED EVERY MINUTE
fn manage_brightness_cycle(state: &AppState, monthly_times: &MonthlyTimes) {
    let current_minutes = get_current_local_time();
    let (month, day) = get_current_month_day();
    let (sunrise, sunset) = get_smoothed_day_times(monthly_times, month, day);
    
    // Calculate Kelvin value based on time of day
    let kelvin = if current_minutes >= sunset || current_minutes < sunrise {
        // Night: fixed 4500K
        4500
    } else {
        let day_length = sunset - sunrise;
        if day_length == 0 {
            // Avoid division by zero
            5500
        } else {
            let half_day = day_length / 2;
            let midpoint = sunrise + half_day;
            
            if current_minutes <= midpoint {
                // Morning: gradually increase from 4500K to 6500K
                4500 + (current_minutes - sunrise) * 2000 / half_day
            } else {
                // Afternoon: gradually decrease from 6500K to 4500K
                6500 - (current_minutes - midpoint) * 2000 / half_day
            }
        }
    };
    
    // Limit values between 4500 and 6500
    let kelvin = kelvin.clamp(4500, 6500);
    
    // Use integrated xsct function instead of external command
    if let Err(e) = xsct_set_temperature(kelvin) {
        if state.verbose && !state.daemon {
            eprintln!("Error setting temperature: {}", e);
        }
    } else if state.verbose && !state.daemon {
        println!("Setting to {}K at {} (sunrise: {}, sunset: {})",
                 kelvin,
                 format_time(current_minutes),
                 format_time(sunrise),
                 format_time(sunset));
    } else if !state.daemon {
        // Even in non-verbose mode, show minimal feedback
        println!("[{}] {}K",
                 format_time(current_minutes),
                 kelvin);
    }
    
    io::stdout().flush().unwrap();
}

// Display help
fn display_help() {
    println!("Usage: colorwarm [options]");
    println!("Options:");
    println!("  -v, --verbose  : Display execution details");
    println!("  -d, --daemon   : Run in background (daemon mode)");
    println!("  -h, --help     : Display this help");
    println!("");
    println!("Automatically manages screen temperature according to seasons:");
    println!("- Night: fixed 4500K");
    println!("- Day: progressive variation between 4500K and 6500K");
    println!("- Automatically detects location from system timezone");
    println!("- Uses smoothed sunrise/sunset times adjusted for detected timezone");
    println!("- Supports over 300 cities and timezones worldwide");
    println!("- Includes integrated xsct functionality (no external dependency)");
}

// Simple non-blocking ESC key check
fn check_esc_key() -> bool {
    use termios::{Termios, tcsetattr, TCSANOW, ICANON, ECHO};
    
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    
    if let Ok(mut termios) = Termios::from_fd(fd) {
        let original = termios.clone();
        
        // Set non-blocking mode
        termios.c_lflag &= !(ICANON | ECHO);
        termios.c_cc[termios::VMIN] = 0;
        termios.c_cc[termios::VTIME] = 0;
        
        if tcsetattr(fd, TCSANOW, &termios).is_ok() {
            let mut buffer = [0; 1];
            let has_esc = io::stdin().read(&mut buffer).is_ok_and(|n| n > 0 && buffer[0] == ESC_KEY);
            
            // Restore settings
            let _ = tcsetattr(fd, TCSANOW, &original);
            return has_esc;
        }
    }
    
    false
}

// xsct standalone function (for direct xsct command emulation)
fn xsct_standalone() -> Result<(), Box<dyn std::error::Error>> {
    let matches = ClapCommand::new("xsct")
        .version(XSCT_VERSION)
        .about("X11 set color temperature")
        .arg(
            Arg::new("temperature")
                .help("Color temperature (0 resets to default 6500K)")
                .index(1),
        )
        .arg(
            Arg::new("brightness")
                .help("Brightness value (0.0 to 1.0)")
                .index(2),
        )
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .help("Display usage information")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Display debugging information")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("delta")
                .short('d')
                .long("delta")
                .help("Consider temperature and brightness as relative shifts")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("toggle")
                .short('t')
                .long("toggle")
                .help("Toggle between 'day' and 'night' mode")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("screen")
                .short('s')
                .long("screen")
                .help("Only select screen specified by zero-based index")
                .value_name("N"),
        )
        .arg(
            Arg::new("crtc")
                .short('c')
                .long("crtc")
                .help("Only select CRTC specified by zero-based index")
                .value_name("N"),
        )
        .get_matches();

    let fhelp = matches.get_flag("help");
    let fdebug = matches.get_flag("verbose");
    let fdelta = matches.get_flag("delta");
    let toggle = matches.get_flag("toggle");

    let screen_specified = matches
        .get_one::<String>("screen")
        .map(|s| s.parse::<usize>().unwrap_or(usize::MAX));
    let crtc_specified = matches
        .get_one::<String>("crtc")
        .map(|s| s.parse::<i32>().unwrap_or(-1))
        .unwrap_or(-1);

    let temp_arg = matches
        .get_one::<String>("temperature")
        .map(|s| s.parse::<i32>().unwrap_or(DELTA_MIN))
        .unwrap_or(DELTA_MIN);
    let brightness_arg = matches
        .get_one::<String>("brightness")
        .map(|s| s.parse::<f64>().unwrap_or(DELTA_MIN as f64))
        .unwrap_or(DELTA_MIN as f64);

    if fhelp {
        print_xsct_usage();
        return Ok(());
    }

    let (conn, _) = RustConnection::connect(None)?;
    let screens = conn.setup().roots.len();

    if let Some(screen) = screen_specified {
        if screen >= screens {
            eprintln!("ERROR! Invalid screen index: {}!", screen);
            return Ok(());
        }
    }

    let screen_first = screen_specified.unwrap_or(0);
    let screen_last = screen_specified.unwrap_or(screens - 1);

    if toggle {
        for screen in screen_first..=screen_last {
            let temp = get_sct_for_screen(&conn, screen, crtc_specified, fdebug);
            let new_temp = if temp.temp > (TEMPERATURE_NORM - 100) {
                TEMPERATURE_NIGHT
            } else {
                TEMPERATURE_NORM
            };
            sct_for_screen(
                &conn,
                screen,
                crtc_specified,
                TempStatus {
                    temp: new_temp,
                    brightness: temp.brightness,
                },
                fdebug,
            );
        }
    }

    let mut temp = TempStatus {
        temp: temp_arg,
        brightness: if brightness_arg == DELTA_MIN as f64 && !fdelta {
            1.0
        } else {
            brightness_arg
        },
    };

    if temp.temp == DELTA_MIN && !fdelta {
        // Aucun argument, afficher la température estimée pour chaque écran
        for screen in screen_first..=screen_last {
            let current_temp = get_sct_for_screen(&conn, screen, crtc_specified, fdebug);
            println!(
                "Screen {}: temperature ~ {} {}",
                screen, current_temp.temp, current_temp.brightness
            );
        }
    } else {
        if !fdelta {
            // Mode absolu
            if temp.temp == 0 {
                temp.temp = TEMPERATURE_NORM;
            } else {
                bound_temp(&mut temp);
            }
            for screen in screen_first..=screen_last {
                sct_for_screen(&conn, screen, crtc_specified, temp, fdebug);
            }
        } else {
            // Mode delta
            if temp.temp == DELTA_MIN || temp.brightness == DELTA_MIN as f64 {
                eprintln!("ERROR! Temperature and brightness delta must both be specified!");
                return Ok(());
            }
            for screen in screen_first..=screen_last {
                let mut tempd = get_sct_for_screen(&conn, screen, crtc_specified, fdebug);
                tempd.temp += temp.temp;
                tempd.brightness += temp.brightness;
                bound_temp(&mut tempd);
                sct_for_screen(&conn, screen, crtc_specified, tempd, fdebug);
            }
        }
    }

    Ok(())
}

fn print_xsct_usage() {
    println!(
        "Xsct ({})
Usage: colorwarm xsct [options] [temperature] [brightness]
\tIf the argument is 0, xsct resets the display to the default temperature (6500K)
\tIf no arguments are passed, xsct estimates the current display temperature and brightness
Options:
\t-h, --help \t xsct will display this usage information
\t-v, --verbose \t xsct will display debugging information
\t-d, --delta\t xsct will consider temperature and brightness parameters as relative shifts
\t-s, --screen N\t xsct will only select screen specified by given zero-based index
\t-t, --toggle \t xsct will toggle between 'day' and 'night' mode
\t-c, --crtc N\t xsct will only select CRTC specified by given zero-based index",
        XSCT_VERSION
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    // Check if we're running xsct mode
    if args.len() > 1 && (args[1] == "xsct" || args[1].ends_with("/xsct")) {
        if let Err(e) = xsct_standalone() {
            eprintln!("Error: {}", e);
            exit(1);
        }
        return;
    }
    
    // Original colorwarm mode
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    let daemon = args.iter().any(|arg| arg == "--daemon" || arg == "-d");

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        display_help();
        return;
    }

    // Try to detect location from system
    let (timezone, location_name) = match guess_location_from_system() {
        Some((tz, name)) => {
            if verbose {
                println!("Detected timezone: {}", tz);
                println!("Location: {}", name);
            }
            (tz, name)
        },
        None => {
            // Default to Brussels if detection fails
            let default_tz = "Europe/Brussels".to_string();
            let default_name = "Brussels, Belgium (default)".to_string();
            
            if verbose {
                println!("Could not detect timezone, using default: {}", default_tz);
            }
            (default_tz, default_name)
        }
    };

    // Initialize monthly times adjusted for detected timezone
    let monthly_times = MonthlyTimes::new_for_timezone(&timezone);

    let state = AppState {
        verbose,
        location_name: location_name.clone(),
        daemon,
    };

    // If daemon mode, detach from terminal
    if daemon {
        let stdout = File::create("/tmp/colorwarm.log").unwrap();
        let stderr = File::create("/tmp/colorwarm.err").unwrap();

        let daemonize = Daemonize::new()
            .pid_file("/tmp/colorwarm.pid");

        match daemonize.start() {
            Ok(()) => {
                // Daemon lancé avec succès
                println!("ColorWarm démarré en mode daemon.");
            }
            Err(e) => {
                eprintln!("Erreur lors du démarrage du daemon: {}", e);
                exit(1);
            }
        }
    }

    println!("ColorWarm v1.30 - Worldwide Timezone Support");
    println!("2025 - Philippe TEMESI");
    println!("https://www.tems.be");
    println!("Timezone: {}", timezone);
    println!("Location: {}", location_name);
    println!("Integrated xsct functionality included");
    println!("");
    if !daemon {
        println!("Press ESC to exit");
        println!("------------------------------------------");
    }
    io::stdout().flush().unwrap();

    // Do first update immediately
    manage_brightness_cycle(&state, &monthly_times);

    // Get current minute
    let mut last_minute = get_current_minute();

    // Main loop
    loop {
        // Check ESC key
        if !daemon && check_esc_key() {
            println!("\nExiting...");
            io::stdout().flush().unwrap();
            break;
        }

        // Wait 100ms
        sleep(Duration::from_millis(100));

        // Get current minute
        let current_minute = get_current_minute();

        // If minute changed, update
        if current_minute != last_minute {
            last_minute = current_minute;
            manage_brightness_cycle(&state, &monthly_times);
        }
    }
}
