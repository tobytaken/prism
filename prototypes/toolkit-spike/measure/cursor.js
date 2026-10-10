console.info("SPIKECURSOR " + workspace.cursorPos.x + "," + workspace.cursorPos.y);
for (const w of workspace.windowList()) { if (w.caption.indexOf("spike)") >= 0) console.info("SPIKEWIN " + w.frameGeometry.x + "," + w.frameGeometry.y + " " + w.frameGeometry.width + "x" + w.frameGeometry.height); }
