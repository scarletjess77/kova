# winget

`0.1.14/` is the first manifest for the [winget-pkgs](https://github.com/microsoft/winget-pkgs)
repository (`winget validate --manifest packaging/winget/0.1.14` passes). To submit it:

```
wingetcreate submit packaging/winget/0.1.14 --token <GitHub token>
```

or copy the folder to `manifests/s/scarletjess77/Kova/0.1.14/` in a fork of winget-pkgs and
open a pull request. Once it is merged, `winget install scarletjess77.Kova` works, and the
`winget` workflow submits each later release by itself when the `WINGET_TOKEN` secret is set.
