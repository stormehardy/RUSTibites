RUSTibites ALPHA  -  a Rust re-implementation of The Bibites, for finding what does not match the original
====================================================================================================

You need: Windows 10/11 (64-bit) and your own copy of The Bibites (Steam). Nothing to install.

START
  1. Unzip this folder anywhere (not inside "Program Files").
  2. Double-click Play.bat.
     First run only (about 20 seconds): it finds your copy of The Bibites and builds the pictures it needs from it.
     If it cannot find the game it asks you to pick the folder that contains "The Bibites.exe".
  3. Play. Windows may show a SmartScreen warning for an unsigned program: "More info" > "Run anyway".

  Play.bat                      worlds, autosaves and bibites go to the "userdata" folder here (safe, separate from your game;
                                the Library lists them first, then your game's own bibites).
  Play-with-my-game-saves.bat   uses the original game's own folders so you can load YOUR saves. This is an alpha: copy your saves first.

WHAT TO LOOK FOR  (the whole point)
  Run the original game and this one side by side and compare. Anything that looks different or behaves differently is a bug we want:
    - a panel, button, slider, tooltip or hotkey that does something else (or nothing)
    - colours, sizes, fonts, spacing, text, positions
    - the simulation: bibites that behave differently on the same save or scenario, population curves, species, energy numbers
    - saves that will not load, or that the original game cannot load after this one wrote them
    - crashes, freezes, slowness
  Known gaps are listed in KNOWN_GAPS.txt - no need to report those.

HOW TO REPORT  (please keep it short and concrete)
  1. Press F12 in the window: it saves a picture of what you see to the "screenshots" folder. Take the same view in the original game.
  2. Double-click Make-Bug-Report.bat. It puts a small zip on your Desktop (logs, settings, your screenshots, machine info, and what Windows
     recorded if the program crashed).
  3. Send the zip and the two pictures with: what you did, what the original does, what this does. The version is in VERSION.txt.

IF THE WINDOW CLOSES BY ITSELF (a crash)
  Run Make-Bug-Report.bat right away and send the zip: the log ends with an "exit code" line and Windows' crash record names what failed.
  Please also say: what you had just done (opened a new world? loaded a save? how many seconds in?), whether it happens every time, and how much memory
  the PC has. It helps to try these two, one at a time, from a command prompt in this folder, and say which one (if any) stops the crash:
      Play.bat -ViewerArgs --no-gpu-hue-shader      (the graphics card's colour program is not used: the bibites' colours are made on the processor, slower with many bibites)
      Play.bat -ViewerArgs --no-overlap             (the simulation does not run beside the drawing)

USEFUL
  Space pause | Esc closes panels / opens the menu | 1-4 inspector tabs | R random bibite | F12 screenshot | H hide UI | F5 quick save
  The title menu's background video needs ffmpeg on your PATH (optional; without it the background is dark).
  Fullscreen is off by default here (Settings > Graphics turns it on, as in the game).

PRIVACY
  Nothing is sent anywhere. The program reads your game's files (read-only) and, if you use the second launcher, your saves.
  It also reads the game's settings from the Windows registry (HKCU\Software\The Bibites) to show your own options; it never writes there.

LICENSE
  The program is MIT / Apache-2.0 (LICENSE-*). This package contains no game art: the pictures are built on your machine from your own
  copy of The Bibites and are not to be shared. The Bibites is by its creator; this is an unofficial fan project made with his permission.
