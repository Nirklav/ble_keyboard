#!/bin/sh
# Cargo runner (.cargo/config.toml): flashes the firmware ELF $1 over the serial port and opens the monitor.
# - The bootloader is the one esp-idf-sys built with sdkconfig.defaults (espflash has its own without
#   rollback), esp-idf-sys copies it next to the profile's build directory.
# - Erasing otadata makes the keyboard boot ota_0, where espflash writes, even if an update ran ota_1.
set -e
elf="$1"
shift
dir=$(dirname "$0")/..
bootloader="$(dirname "$elf")/build/bootloader.bin"
exec espflash flash --monitor \
    --partition-table "$dir/partitions.csv" \
    --bootloader "$bootloader" \
    --target-app-partition ota_0 \
    --erase-parts otadata \
    "$@" "$elf"
