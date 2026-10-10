#!/bin/bash
# usage: theme_test.sh <binary> <tag>
python3 -I measure/mock_portal.py &
sleep 1
"$1" --sim > /tmp/theme-$2.log 2>&1 &
APP=$!
sleep 4; spectacle -b -n -a -o /tmp/theme-$2-a.png
sleep 5; spectacle -b -n -a -o /tmp/theme-$2-b.png
sleep 6; spectacle -b -n -a -o /tmp/theme-$2-c.png
kill $APP; kill %1
