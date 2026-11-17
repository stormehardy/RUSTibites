# Headless / no-visible-windows policy (agents and tooling)

The user works at this machine while agents develop RUSTibites. **Anything we launch must
not put a window on their desktop unless they explicitly asked for it.** A stray game or
viewer window can steal focus, change resolution or be touched by accident.

## Rules

1. **The simulation CLI is headless.** `rustibites run/bench/info/check/braincheck`,
   parity captures, compatibility scans and fuzz tests are console-only: run them with
   `--quiet` where sensible. They may use significant CPU (warn the user in updates) but
   must never open a window.
2. **The installed game is launched hidden only.** Live-game work (parity captures,
   template/scenario acceptance, reference screenshots) must use the existing headless
   harness path (`tools/parity/capture.py` / `tools/roundtrip/` `LiveRuntime` with
   `window="hidden"`, or an equivalent offscreen/hidden window). Never start a normal
   windowed game session from an agent turn.
3. **Reference screenshots**: capture from the hidden window when Unity still renders
   (screen capture of the hidden/offscreen window). If a visible window is genuinely
   unavoidable, **stop and ask the Lead/user first**, run for the shortest possible burst,
   restore window state, and close it immediately.
4. **The viewer (`rustibites-view.exe`) is for the user.** Agents verify rendering with the
   headless `--screenshot` mode, never by leaving the windowed app running. If a windowed
   check is ever required, ask first and close it in the same step.
5. **Always clean up**: after any game/viewer run, stop the process, verify no
   `The Bibites`, `rustibites-view`, `rustibites` (leftover) or BepInEx/ParityDump
   artifacts remain, and check the API ports (8800/17800/17801) are free. Remove any
   transient plugin/instrumentation deployed to the game install and leave the game
   install/AppData byte-identical unless the task explicitly writes a test save.
6. **Never touch user processes.** Only stop processes whose command line points into
   `Z:\RUSTibites\target\...` (our binaries) or the game install we launched ourselves.
   In particular, do not kill unrelated `python`/`pythonw`/Merlin processes.
7. Long-running loops (compat scans, benchmark matrices, evolution runners) must be
   bounded and report progress; they may run in the background but stay headless.

## Quick audit commands

```powershell
# Anything of ours with a window?
Get-Process rustibites-view,rustibites -ErrorAction SilentlyContinue |
  Select-Object Id,ProcessName,MainWindowTitle

# The game still running?
Get-Process | Where-Object { $_.MainWindowTitle -like '*Bibites*' }

# Ports used by the live harness
Get-NetTCPConnection -State Listen -LocalPort 8800,17800,17801 -ErrorAction SilentlyContinue

# Our CLI processes (headless; command line shows target path)
Get-CimInstance Win32_Process |
  Where-Object { $_.CommandLine -like '*RUSTibites\target*' } |
  Select-Object ProcessId,Name,CommandLine
```

If an audit shows a leftover **viewer window**, close only that PID
(`Stop-Process -Id <pid>`); game processes from our harness must be stopped through the
same harness or a targeted PID kill after checking its command line.

## Agent checklist (each turn that touches the game or viewer)

- [ ] Launched headless / hidden window only
- [ ] No window left on the user's desktop (`MainWindowTitle` empty for our processes)
- [ ] Process stopped, plugin/instrumentation removed
- [ ] Ports 8800/17800/17801 free
- [ ] Game install and AppData unchanged (except documented test artifacts)
