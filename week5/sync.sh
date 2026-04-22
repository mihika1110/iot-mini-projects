#!/bin/bash

# Ensure correct usage
if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <username> <hostname>"
    echo "Example: $0 sahil davinci.local"
    exit 1
fi

USER=$1
HOST=$2
DEST_DIR="whyblue"

echo "Syncing whyblue codebase to $USER@$HOST:~/$DEST_DIR..."

# We use rsync instead of raw scp because rsync allows us to exclude the heavy Rust 'target/' directories.
# A raw `scp -r` would attempt to copy gigabytes of locally compiled binaries, whereas this instantly syncs only source code.
rsync -avz --exclude='target/' --exclude='.git/' --exclude='*.sock' ./ "$USER@$HOST:~/$DEST_DIR/"

echo "Sync complete!"
