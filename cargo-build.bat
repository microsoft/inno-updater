@echo off

SET cargoargs=
IF "%~1"=="--release" SET cargoargs=--release --target="i686-pc-windows-msvc"

cargo build %cargoargs%
CALL .\enable-visual-styles.bat %~1