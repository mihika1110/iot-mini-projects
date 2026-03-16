#!/usr/bin/env bash

help() {
    cat << EOF
    Usage: ./reflash.sh -p [PORT] -i [DEVICE_ID]    
EOF
}

NARGS=$#

parse_arg() {
    flag=$1
    value=$2

    case $flag in
        -h|--help)
            help
            exit;;
        -p|--port)
            F_PORT=$2
            ;;
        -i|--id)
            F_ID=$2
            ;;
        *)
            echo "Invalid Argument!"
            help
            exit;; 
    esac
}

if [[ $NARGS -eq 4 ]]; then
    parse_arg $1 $2
    parse_arg $3 $4
elif [[ $NARGS -eq 2 ]]; then
    parse_arg $1 $2
else
    parse_arg
fi

# Set the default value for the device ID
if [[ -z F_ID ]]; then
    F_ID=1
fi

HOST_IP=$(ip addr | grep -A5 wlan0 | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+' | head -n1)
echo "Host IP: $HOST_IP"

sed -i "s/s-[0-1]*/s-00$F_ID/g" node_firmware.ino 
sed -i "s/[0-9]*\.[0-9]*\.[0-9]*\.[0-9]*/$HOST_IP/g" node_firmware.ino

arduino-cli compile --fqbn esp32:esp32:esp32
arduino-cli upload -p "$F_PORT" --fqbn esp32:esp32:esp32
arduino-cli monitor -p "$F_PORT" --config 115200
