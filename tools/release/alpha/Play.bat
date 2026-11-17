@echo off
rem RUSTibites alpha launcher: first run builds the art from your own copy of The Bibites, then starts the viewer.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0play.ps1" %*
if errorlevel 1 pause
