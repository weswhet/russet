---
title: Command-line reference
description: The syntax, option parsing, and verbs of the russet command that Russet installs.
---

Russet installs a single command, `russet`, that accepts the same verbs and
options as Python AutoPkg 3.0.0. This page describes the general syntax and
lists every verb.

## Syntax

Every command starts with a verb:

```sh
russet VERB [OPTIONS] [ARGUMENTS]
```

Replace the following:

- `VERB`: the action to perform, such as `run` or `repo-add`.
- `OPTIONS`: options for that verb.
- `ARGUMENTS`: the recipes, recipe repositories, or other items that the verb
  works on.

## Get help

To print the help for one verb, run the verb with `--help` or `-h`:

```sh
russet run --help
```

The verb's help prints to standard output, and the command exits with status
`0`.

To list the verbs, run `russet help`, `russet --help`, or `russet` with no
arguments. The list prints to standard output, and the command exits with
status `1`. `russet help VERB` prints the same list instead of the help for
`VERB`.

## Option parsing

Russet parses options the same way that Python AutoPkg does:

- You can shorten a long option to any prefix that matches only one option.
  For example, `--with-i` means `--with-identifiers`.
- A long option that takes a value accepts either `--option VALUE` or
  `--option=VALUE`.
- You can group short options, such as `-vv` or `-vc`, and attach a value to
  a short option, such as `-kNAME=VALUE`.
- `--` ends the options. Russet treats everything after it as an argument.
- If Russet can't parse an option, it prints the verb's usage and an error,
  such as `russet: error: no such option: --bogus`, and exits with status
  `2`.

## Preferences option

Most verbs accept `--prefs FILE`. Russet reads your platform preferences
first and then applies the values from `FILE`. If the verb changes
preferences, such as `repo-add`, Russet saves the change to `FILE`. For
details, see [Preferences](/reference/preferences/).

## Verbs

The following verbs are available:

| Verb | Description |
| --- | --- |
| [`audit`](/reference/cli/russet-audit/) | Checks recipes for risky patterns without running them. |
| [`clear-cache`](/reference/cli/russet-clear-cache/) | Removes cached files for a recipe or for all recipes. |
| [`generate-recipe-map`](/reference/cli/russet-generate-recipe-map/) | Builds or rebuilds the recipe map file. |
| [`help`](/reference/cli/russet-help/) | Lists the verbs. |
| [`info`](/reference/cli/russet-info/) | Shows your preferences or details about a recipe. |
| [`install`](/reference/cli/russet-install/) | Runs install recipes. |
| [`list-processors`](/reference/cli/russet-list-processors/) | Lists the processors that Russet implements. The alias `processor-list` does the same thing. |
| [`list-recipes`](/reference/cli/russet-list-recipes/) | Lists the recipes in your search and override folders. |
| [`make-override`](/reference/cli/russet-make-override/) | Creates a recipe override. |
| [`new-recipe`](/reference/cli/russet-new-recipe/) | Creates a recipe from a template. |
| [`processor-info`](/reference/cli/russet-processor-info/) | Shows a processor's description and variables. |
| [`repo-add`](/reference/cli/russet-repo-add/) | Clones recipe repositories and adds them to the search path. |
| [`repo-delete`](/reference/cli/russet-repo-delete/) | Removes recipe repositories. |
| [`repo-list`](/reference/cli/russet-repo-list/) | Lists the recipe repositories that you added. The alias `list-repos` does the same thing. |
| [`repo-update`](/reference/cli/russet-repo-update/) | Updates recipe repositories with Git. |
| [`run`](/reference/cli/russet-run/) | Runs recipes. |
| [`search`](/reference/cli/russet-search/) | Searches for recipes in the AutoPkg organization on GitHub. |
| [`update-trust-info`](/reference/cli/russet-update-trust-info/) | Records the current state of a recipe override's parent recipes. |
| [`verify-trust-info`](/reference/cli/russet-verify-trust-info/) | Checks that a recipe override's parent recipes haven't changed. |
| [`version`](/reference/cli/russet-version/) | Prints the AutoPkg compatibility version. |

## Output

Russet prints normal output to standard output and warnings and errors to
standard error. On Windows, text output uses Windows line endings, a carriage return followed
by a line feed. Property list
output is the same on every platform.

## Related pages

- [Exit codes](/reference/exit-codes/)
- [Preferences](/reference/preferences/)
- [Environment variables](/reference/environment-variables/)
