@echo off

SET target=debug
IF "%~1"=="--release" SET target=release

.\tools\mt.exe -nologo -manifest main.manifest -outputresource:"target\%target%\inno_updater.exe;#1"