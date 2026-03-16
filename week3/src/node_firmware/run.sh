#!/bin/bash

if [[ $# -ne 1 ]]; then
    echo "Missing username"
fi

USERNAME=$1

if [[ ! -f /etc/mosquitto/passwd ]]; then
    echo "Creating password file: /etc/mosquitto/passwd with username: $USERNAME"
    sudo mosquitto_passwd -c /etc/mosquitto/passwd $USERNAME
    sudo chown $(whoami) /etc/mosquitto/passwd
fi

mosquitto -c mosquitto.conf
