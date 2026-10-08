---
title: russet help
description: List the verbs that the russet command accepts.
---

`russet help` prints the list of verbs with a short description of each one.

## Syntax

```sh
russet help
```

## Description

`russet help` prints the verb list to standard output and exits with status
`1`, as Python AutoPkg does. Russet ignores any arguments, so
`russet help VERB` prints the same list. To get help for one verb, run
`russet VERB --help`.

Running `russet` with no arguments prints the same list. Running
`russet --help`, `russet -h`, or `russet` with an unknown verb prints the
list followed by an error, such as `Error: unknown verb: --help`, and also
exits with status `1`.

## Exit status

| Status | Meaning |
| --- | --- |
| `1` | Russet printed the verb list. |

## Examples

To list the verbs, run the following command:

```sh
russet help
```

To print the help for `russet run`, run the following command:

```sh
russet run --help
```

## Related pages

- [Command-line reference](/reference/cli/)
