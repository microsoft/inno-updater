@echo off

SET target=debug
IF "%~1"=="--release" SET target=release

CALL .\cargo-build.bat %*
.\target\%target%\inno_updater.exe
