ColorWarm

Automatically adjust your screen color temperature based on time of day and season.

Overview

ColorWarm is a Rust application that protects your eyes and sleep cycle by smoothly adjusting your screen's color temperature throughout the day. It transitions from cool daylight (6500K) to warm evening light (4500K) based on actual sunrise and sunset times for your location, with seasonal variations built in.

Unlike simple on/off screen filters, ColorWarm provides progressive, natural transitions that follow the sun's cycle.
Key Features

    Automatic Location Detection – Detects your timezone and selects the nearest city from a database of 300+ worldwide locations

    Seasonal Adaptation – Sunrise/sunset tables evolve throughout the year, with daily interpolation

    Smooth Transitions – Color temperature changes gradually, not abruptly

    Daemon Mode – Runs quietly in the background, starts automatically with your session

    Integrated xsct – Built-in X11 color temperature control, no external dependencies

    Zero Configuration – Works out of the box with sensible defaults

How It Works

ColorWarm calculates smoothed sunrise and sunset times for your precise day of the year, then:

    Night – Fixed warm temperature (4500K)

    Morning – Gradually warms up to daylight (4500K → 6500K)

    Afternoon – Gradually cools back to evening (6500K → 4500K)

All calculations use local time and account for approximate longitude offsets derived from your timezone.
Installation
Prerequisites

    Linux with X11 and RandR extension

    Rust and Cargo (for building from source)

Build from source

git clone https://github.com/philtems/colorwarm.git

cd colorwarm

cargo build --release

sudo cp target/release/colorwarm /usr/local/bin/

Usage
Automatic mode (recommended)

# Run once (interactive, press ESC to exit)
colorwarm

# Run in verbose mode
colorwarm -v

# Run as a background daemon
colorwarm -d

# Display help
colorwarm -h

Manual mode (built-in xsct)

ColorWarm includes a complete implementation of xsct (X Set Color Temperature):
bash

# Set specific temperature
colorwarm xsct 4500

# Set temperature and brightness
colorwarm xsct 5000 0.8

# Reset to default (6500K)
colorwarm xsct 0

# Display current temperature
colorwarm xsct

# Toggle between day/night mode
colorwarm xsct -t

# For all xsct options
colorwarm xsct -h

Auto-start with your desktop

Add to your startup applications (GNOME, KDE, XFCE, etc.):

/usr/local/bin/colorwarm -d

Supported Locations

ColorWarm includes timezone-based location data for:

    Europe – All major cities from Lisbon to Moscow

    North America – US, Canada, Mexico, Caribbean

    South America – All countries

    Asia – From Middle East to Japan

    Africa – All timezones

    Oceania – Australia, New Zealand, Pacific Islands

If your exact city isn't listed, the application falls back to your detected timezone region with appropriate longitude adjustment.
Why ColorWarm?

    Eye strain reduction – Especially during evening work sessions

    Better sleep – Reduced blue light exposure before bedtime

    Natural experience – Your screen follows the sun, not an arbitrary schedule

    Resource efficient – Written in Rust, minimal CPU/memory footprint

    No cloud services – Works entirely offline, no privacy concerns

Technical Details

    Written in Rust for safety and performance

    Uses X11RB for direct RandR CRTC gamma control

    No external dependencies for color management – implements its own gamma ramps

    Daemon mode uses daemonize crate

    Timezone detection via /etc/timezone or /etc/localtime

Command Line Options
Option	Description

-v, --verbose	Display detailed information about current settings
-d, --daemon	Run in background, log to /tmp/colorwarm.log
-h, --help	Show help message


Philippe TEMESI
https://www.tems.be

Acknowledgments

Inspired by the original xsct utility by Ted Unangst
