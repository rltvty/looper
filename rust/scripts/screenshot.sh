#!/bin/bash
# Screenshot utility for debugging the looper app
# Usage: ./scripts/screenshot.sh [filename]
#
# Captures a screenshot of the MIDI Looper window (or full screen as fallback)

# Get the directory where this script lives
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

SCREENSHOTS_DIR="$PROJECT_DIR/screenshots"
mkdir -p "$SCREENSHOTS_DIR"

if [ -n "$1" ]; then
    FILENAME="$SCREENSHOTS_DIR/$1.png"
else
    TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
    FILENAME="$SCREENSHOTS_DIR/looper_$TIMESTAMP.png"
fi

# Get window ID for "MIDI Looper" window using AppleScript
WINDOW_ID=$(osascript -e '
tell application "System Events"
    set looperProcess to first process whose name contains "looper"
    set looperWindow to first window of looperProcess
    return id of looperWindow
end tell
' 2>/dev/null)

if [ -n "$WINDOW_ID" ] && [ "$WINDOW_ID" != "" ]; then
    # Capture specific window by ID
    screencapture -l "$WINDOW_ID" -x "$FILENAME" 2>/dev/null
    if [ -f "$FILENAME" ]; then
        echo "Window screenshot saved: $FILENAME"
        exit 0
    fi
fi

# Fallback: capture entire screen (silent mode)
screencapture -x "$FILENAME"
echo "Full screen captured: $FILENAME"
