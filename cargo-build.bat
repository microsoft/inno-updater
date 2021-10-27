@echo off

SET cargoargs=
IF "%~1"=="--release" SET cargoargs=--release --target="i686-pc-windows-msvc"

cargo build %cargoargs%

SET target=debug
IF "%~1"=="--release" SET target=i686-pc-windows-msvc\release