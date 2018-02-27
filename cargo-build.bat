@echo off

SET cargoargs=
IF "%~1"=="--release" SET cargoargs=--release --target="i686-pc-windows-msvc"

cargo build %cargoargs%

SET target=debug
IF "%~1"=="--release" SET target=i686-pc-windows-msvc\release

.\tools\mt.exe -nologo -manifest resources\main.manifest -outputresource:"target\%target%\inno_updater.exe;#1"
.\tools\rcedit.exe target\%target%\inno_updater.exe --set-icon resources\code.ico