---
title: autopkg list-processors
description: List the processors that Russet implements.
---

`autopkg list-processors` prints the name of every processor that Russet
implements. The alias `autopkg processor-list` does the same thing.

## Syntax

```sh
autopkg list-processors
```

## Description

Russet prints one processor name for each line, sorted by name. It lists 59
names: the 46 AutoPkg built-in processors, 12 processors from the
`autopkg/recipes` recipe repository that Russet implements natively, and one
alias for a shared recipe processor.

Russet ignores arguments, including `--prefs`.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Accepted for compatibility. Russet ignores it. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the list. |
| `2` | Russet couldn't parse an option. |

## Examples

To count the processors that your installation implements, run the following
command:

```sh
autopkg list-processors | wc -l
```

## Related pages

- [Processors](/reference/processors/)
- [`autopkg processor-info`](/reference/cli/autopkg-processor-info/)
