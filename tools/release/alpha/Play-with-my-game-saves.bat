@echo off
rem Same as Play.bat, but worlds, autosaves and bibites go to the original game's own folders (AppData\LocalLow\The Bibites).
rem Use it to load your existing saves. A bug in this alpha could write a bad file there: copy your saves somewhere first.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0play.ps1" -GameSaves %*
if errorlevel 1 pause
