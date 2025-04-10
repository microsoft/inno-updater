# inno-updater

[![Build Status](https://dev.azure.com/monacotools/Monaco/_apis/build/status%2FMisc%2Finno-updater?repoName=microsoft%2Finno-updater&branchName=main)](https://dev.azure.com/monacotools/Monaco/_build/latest?definitionId=623&repoName=microsoft%2Finno-updater&branchName=main)

Helper utility to enable background updates for VS Code in Windows

- https://github.com/Microsoft/vscode
- https://code.visualstudio.com/

## Integration

⭐️ To create a new release, simply push a new tag; this will kick off a [build](https://dev.azure.com/vscode/Inno%20Updater/_build?definitionId=25&_a=summary) and publish a [release](https://github.com/microsoft/inno-updater/releases).

⭐️ To integrate a release of `inno-updater` in VS Code, simply extract the release archive to [`build/win32`](https://github.com/microsoft/vscode/tree/main/build/win32).

## Contributing

This project welcomes contributions and suggestions. Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit https://cla.microsoft.com.

When you submit a pull request, a CLA-bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., label, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.
