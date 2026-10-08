---
title: russet install
description: Run one or more install recipes by item name.
---

`russet install` runs install recipes, which install software on the computer
that runs them.

## Syntax

```sh
russet install [OPTIONS] [ITEM ...]
```

Replace the following:

- `OPTIONS`: any of the options that [`russet run`](/reference/cli/russet-run/#options)
  accepts.
- `ITEM`: the name of the software to install, such as `Firefox`, or the name
  of an install recipe, such as `Firefox.install`.

## Description

`russet install` works like `russet run`, except for how it treats the names
that you pass:

- A name without an extension gets `.install` added. For example, `Firefox`
  becomes `Firefox.install`.
- A name that ends in `.install` stays the same.
- A name with any other extension isn't an install recipe. Russet prints
  `Can't install with a non-install recipe: NAME` and skips it. If no names
  remain, Russet prints the usage and exits with status `255`.

Russet doesn't change the names in a recipe list that you pass with
`--recipe-list`.

Install recipes use the `Installer` and `InstallFromDMG` processors, which
ask the `russet-installd` helper service to install software as root. These
processors work only on macOS. For details, see
[How Russet works](/concepts/how-russet-works/#helper-services).

Russet finds, validates, and runs recipes the same way as `russet run`. For
details, see
[How Russet finds recipes](/reference/cli/russet-run/#how-russet-finds-recipes).

## Options

`russet install` accepts the same options as `russet run`. For the full
table, see [`russet run` options](/reference/cli/russet-run/#options).

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Every recipe ran without an error. |
| `1` | Validation failed, so no recipe ran. |
| `2` | Russet couldn't parse an option. |
| `70` | At least one recipe failed while it ran. |
| `255` | No install recipes remained to run. On Windows, the status is `-1`. |

## Examples

If you have a recipe override named `Firefox.install`, the following command
runs it:

```sh
russet install Firefox
```

To install the items in a recipe list and print each processor's messages,
run the following command:

```sh
russet install -v --recipe-list ~/install-list.txt
```

## Related pages

- [`russet run`](/reference/cli/russet-run/)
- [Run recipes](/guides/run-recipes/)
- [How Russet works](/concepts/how-russet-works/)
