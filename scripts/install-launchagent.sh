#!/bin/bash
# Installs the LaunchAgent so time-tracker starts on login.
# Run once after building: ./scripts/install-launchagent.sh

set -e

APP_PATH="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/target/release/bundle/macos/time-tracker.app"
BINARY="$APP_PATH/Contents/MacOS/time-tracker"
PLIST="$HOME/Library/LaunchAgents/com.timetracker.app.plist"

if [ ! -f "$BINARY" ]; then
  echo "Binary not found at $BINARY"
  echo "Run 'npm run tauri build' first."
  exit 1
fi

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.timetracker.app</string>
    <key>ProgramArguments</key>
    <array>
        <string>${BINARY}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
    <key>StandardOutPath</key>
    <string>${HOME}/Library/Logs/time-tracker.log</string>
    <key>StandardErrorPath</key>
    <string>${HOME}/Library/Logs/time-tracker.log</string>
</dict>
</plist>
EOF

launchctl unload "$PLIST" 2>/dev/null || true
launchctl load "$PLIST"

echo "LaunchAgent installed. time-tracker will start on next login."
echo "To start it now: launchctl start com.timetracker.app"
