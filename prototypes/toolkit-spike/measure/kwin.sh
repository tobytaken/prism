#!/bin/bash
# usage: kwin.sh minimize|restore  -- acts only on windows whose caption contains "spike)"
V=$([ "$1" = minimize ] && echo true || echo false)
F=measure/kwin-$1.js
cat > $F <<JS
for (const w of workspace.windowList()) { if (w.caption.indexOf("spike)") >= 0) { w.minimized = $V; if (!$V) workspace.activeWindow = w; } }
JS
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript spike-$1 >/dev/null 2>&1
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript $F spike-$1 >/dev/null
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
sleep 0.5
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript spike-$1 >/dev/null
