---
title: autopkg version
description: Print the AutoPkg compatibility version.
---

`autopkg version` prints the version of AutoPkg that Russet is compatible
with.

## Syntax

```sh
autopkg version
```

## Description

Russet prints `3.0.0`, the AutoPkg compatibility version. Recipes that set
`MinimumVersion` compare against this version, and recipes receive it in the
`AUTOPKG_VERSION` variable.

`autopkg version` doesn't print the Russet release version. To find the
release that you installed, read `RELEASE.json` in the release archive that
you installed from. An archive that you build from source doesn't have a
release version. For details, see
[How Russet works](/concepts/how-russet-works/#versions).

Russet ignores any arguments, including `--help`.

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the version. |

## Examples

To print the compatibility version, run the following command:

```sh
autopkg version
```

The output is the following:

```text
3.0.0
```

## Related pages

- [Compatibility with AutoPkg](/concepts/compatibility/)
- [Install Russet](/get-started/install/)
