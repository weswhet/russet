---
title: autopkg help
description: List the verbs that the autopkg command accepts.
---

`autopkg help` prints the list of verbs with a short description of each one.

## Syntax

```sh
autopkg help
```

## Description

`autopkg help` prints the verb list to standard output and exits with status
`1`, as Python AutoPkg does. Russet ignores any arguments, so
`autopkg help VERB` prints the same list. To get help for one verb, run
`autopkg VERB --help`.

Running `autopkg` with no arguments prints the same list. Running
`autopkg --help`, `autopkg -h`, or `autopkg` with an unknown verb prints the
list followed by an error, such as `Error: unknown verb: --help`, and also
exits with status `1`.

## Exit status

| Status | Meaning |
| --- | --- |
| `1` | Russet printed the verb list. |

## Examples

To list the verbs, run the following command:

```sh
autopkg help
```

To print the help for `autopkg run`, run the following command:

```sh
autopkg run --help
```

## Related pages

- [Command-line reference](/reference/cli/)
