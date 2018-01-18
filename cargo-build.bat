@echo off

SET cargoargs=
IF "%~1"=="--release" SET cargoargs=--release

cargo build %cargoargs%
CALL .\enable-visual-styles.bat %~1