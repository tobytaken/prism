# usage: measure.py <binary> <tag> [app args...]
# Launches the app, waits, samples memory/CPU/frames/VRAM; optional selftest and minimize phases.
import os, re, subprocess, sys, time
binary, tag, args = sys.argv[1], sys.argv[2], sys.argv[3:]
selftest = os.environ.get("MEASURE_SELFTEST") == "1"
minimize = int(os.environ.get("MEASURE_MINIMIZE", "0"))
settle, window = int(os.environ.get("MEASURE_SETTLE", "20")), int(os.environ.get("MEASURE_WINDOW", "60"))
env = dict(os.environ, SPIKE_STATS="1")
if selftest:
    env.update(SPIKE_SELFTEST="1", SPIKE_SELFTEST_DELAY=str(int(os.environ.get("MEASURE_SETTLE", "20")) + 62), SPIKE_SELFTEST_LOOPS="7")
log = open(f"/tmp/measure-{tag}.log", "w")
p = subprocess.Popen([binary, *args], env=env, stdout=log, stderr=subprocess.STDOUT)
pid, tck = p.pid, os.sysconf("SC_CLK_TCK")

def mem():
    r = {}
    for line in open(f"/proc/{pid}/smaps_rollup"):
        k, *v = line.split()
        if k.rstrip(":") in ("Rss", "Pss", "Private_Clean", "Private_Dirty", "Pss_Anon", "Pss_File", "Pss_Shmem"):
            r[k.rstrip(":")] = int(v[0]) / 1024
    st = open(f"/proc/{pid}/status").read()
    r["Uss"] = r["Private_Clean"] + r["Private_Dirty"]
    r["HWM"] = int(re.search(r"VmHWM:\s+(\d+)", st).group(1)) / 1024
    r["Threads"] = int(re.search(r"Threads:\s+(\d+)", st).group(1))
    return r

def cpu():
    f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(f[11]) + int(f[12])) / tck

def frames():
    m = re.findall(r"frames=(\d+)", open(f"/tmp/measure-{tag}.log").read())
    return int(m[-1]) if m else 0

def vram():
    out = subprocess.run(["nvidia-smi"], capture_output=True, text=True).stdout
    for line in out.splitlines():
        if re.search(rf"\s{pid}\s", line):
            m = re.search(r"(\d+)MiB", line)
            return f"{m.group(1)} MiB ({line.split()[4]})" if m else "?"
    return "not an NVIDIA client"

def gpu_libs():
    maps = open(f"/proc/{pid}/maps").read()
    libs = sorted({os.path.basename(l.split()[-1]) for l in maps.splitlines() if re.search(r"nvidia|radeon|amdgpu|libvulkan|libEGL|libGLX|mesa|gallium|lvp|llvm", l.split()[-1] if len(l.split()) > 5 else "")})
    return libs

def window_stats(secs, label):
    c0, f0, t0 = cpu(), frames(), time.time()
    time.sleep(secs)
    c1, f1, t1 = cpu(), frames(), time.time()
    m = mem()
    print(f"  {label:<10} cpu={100*(c1-c0)/(t1-t0):5.2f}%  frames/10s≈{(f1-f0)/((t1-t0)/10):6.1f}  "
          f"RSS={m['Rss']:6.1f}  PSS={m['Pss']:6.1f}  USS={m['Uss']:6.1f}  anonPSS={m['Pss_Anon']:6.1f}  HWM={m['HWM']:6.1f} MiB  threads={m['Threads']}")
    return m

time.sleep(settle)
subprocess.run(["qdbus6", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.loadScript", "measure/cursor.js", "spike-cur"], capture_output=True)
subprocess.run(["qdbus6", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.start"], capture_output=True)
time.sleep(0.5)
subprocess.run(["qdbus6", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting.unloadScript", "spike-cur"], capture_output=True)
j = subprocess.run(["journalctl", "--user", "--since", "-5s"], capture_output=True, text=True).stdout
cur = re.findall(r"SPIKECURSOR (\d+),(\d+)", j); win = re.findall(r"SPIKEWIN (-?\d+),(-?\d+) (\d+)x(\d+)", j)
over = "?"
if cur and win:
    cx_, cy_ = map(int, cur[-1]); wx, wy, ww, wh = map(int, win[-1])
    over = wx <= cx_ < wx + ww and wy <= cy_ < wy + wh
print(f"[{tag}] {binary} {' '.join(args)}   pointer-over-window={over}", flush=True)
print(f"  gpu: {vram()}   libs: {', '.join(gpu_libs())}")
window_stats(window, "idle")
if selftest:
    time.sleep(2)
    window_stats(20, "dragging")
if minimize:
    subprocess.run(["measure/kwin.sh", "minimize"])
    window_stats(minimize, "minimized")
    subprocess.run(["measure/kwin.sh", "restore"])
    time.sleep(3)
    subprocess.run(["spectacle", "-b", "-n", "-a", "-o", f"/tmp/restored-{tag}.png"])
    window_stats(20, "restored")
p.terminate()
p.wait()
