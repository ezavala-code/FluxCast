#!/bin/sh
set -eu
cd "$(dirname "$0")/apps/android"
exec ./gradlew "$@"
