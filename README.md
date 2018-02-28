# inno-updater

[![Build status](https://ci.appveyor.com/api/projects/status/q3a8vi08gsgqc478?svg=true)](https://ci.appveyor.com/project/VSCode/inno-updater)

Helper utility to enable background updates for VS Code in Windows

- https://github.com/Microsoft/vscode
- https://code.visualstudio.com/

## Development

Please use the provided `cargo-build` and `cargo-run` commands instead of the default `cargo build` and `cargo run` ones. Note that `--release` will build for the `i686-pc-windows-msvc` target, which is the correct target to ship with VS Code.

## Integration

⭐️ To create a new release, simply push a new tag; this will kick off a [build](https://ci.appveyor.com/project/VSCode/inno-updater) and publish a [release](https://github.com/Microsoft/inno-updater/releases).

⭐️ To integrate a release of `inno-updater` in VS Code, simply extract the release archive to [`build/win32`](https://github.com/Microsoft/vscode/tree/master/build/win32).

## Contributing

This project welcomes contributions and suggestions.  Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit https://cla.microsoft.com.

When you submit a pull request, a CLA-bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., label, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.
