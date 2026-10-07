# installer -query RestartAction

Russet's `russet-installer` crate reproduces this query on Linux. These notes
record the behavior it matches. They were observed with `installer` on macOS
27.0 (build 26A428), using packages built by `pkgbuild` and `productbuild`
on the same system. The tests in `rust/crates/installer` repeat the comparison
on every macOS CI runner.

## Output values

`installer -query RestartAction -pkg PACKAGE -plist` prints a property list
whose `RestartAction` value is one of `None`, `RecommendRestart`,
`RequireLogout`, `RequireRestart`, or `RequireShutdown`.

## Component packages

A component package (one without a `Distribution` file) reports its
`PackageInfo` `postinstall-action` attribute:

| `postinstall-action` | Result |
| --- | --- |
| `none` or missing | `None` |
| `logout` | `RequireLogout` |
| `restart` | `RequireRestart` |
| `shutdown` | `RequireShutdown` |

## Product archives

A product archive (one with a `Distribution` file) reports the most severe
`onConclusion` attribute among its `pkg-ref` elements, in this order:
`None` < `RecommendRestart` < `RequireLogout` < `RequireRestart` <
`RequireShutdown`.

- The components' own `postinstall-action` values are ignored. A product
  whose only component asks for `restart`, but whose `pkg-ref` has
  `onConclusion="None"` or no `onConclusion`, reports `None`.
- Values are compared without regard to case. `productbuild --package`
  writes `onConclusion="RequireLogOut"` for a component whose
  `postinstall-action` is `logout`, and `installer` reports
  `RequireLogout`.
- Unknown values count as `None`.

Not observed: whether `pkg-ref` elements outside the selected choices count.
Russet counts every `pkg-ref`, which can only report a more severe action.

## Bundle packages

`installer` on macOS 27 rejects bundle-style packages with "the package path
specified was invalid", so there's no reference behavior. Russet maps the
documented `IFPkgFlagRestartAction` values: `RecommendedRestart`,
`RequiredLogout`, `RequiredRestart`, and `Shutdown` become
`RecommendRestart`, `RequireLogout`, `RequireRestart`, and
`RequireShutdown`; anything else, including `NoRestart`, is `None`.

## installer -showChoiceChangesXML

Not reproduced. Distribution files can select packages with JavaScript, which
Russet doesn't run, so Munki's `installerChoices` option fails on Linux with
an explanation.
