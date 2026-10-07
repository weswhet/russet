---
title: Exit codes
description: The exit codes that the autopkg command returns and what a script or scheduled job should do with them.
---

A script or scheduled job can use the exit code of the `autopkg` command to
check whether the command succeeded. This page lists the codes and the commands
that return them.

## Exit codes

The `autopkg` command returns the following codes:

| Code | Meaning | Returned by |
| --- | --- | --- |
| `0` | The command succeeded. | Every verb, and `autopkg VERB --help`. |
| `1` | The command failed, and Russet printed the reason to standard error. | Most verbs. For `run` and `install`, this code means that validation failed and no recipe ran. `audit` returns it when a finding reaches the `--fail-on` severity, and `verify-trust-info` returns it when an override fails verification. `autopkg help` and `autopkg` with no verb also return it. |
| `2` | Russet couldn't parse an option. It printed the verb's usage and the error. | Every verb that accepts options. |
| `70` | At least one recipe failed while it ran. Other recipes in the same command might have succeeded. | `run` and `install`. |
| `255` | The arguments were wrong for the verb, such as a missing recipe name. On Windows, the code is `-1`. | `run`, `install`, `audit`, `info`, `make-override`, `new-recipe`, `processor-info`, `repo-add`, `repo-delete`, `repo-update`, `update-trust-info`, and `verify-trust-info`. |

## Handle exit codes in scripts

Treat any exit code other than `0` as a failure. The following codes need
different follow-up:

- **`1` from `run` or `install`:** no recipe ran. Read standard error to find
  the recipe that failed validation, such as a missing recipe, a failed
  trust check on an override, or an unsupported processor.
- **`70` from `run` or `install`:** some recipes ran. Use `--report-plist` to
  save a report, and read its `failures` array to find the recipes that
  failed.
- **`255` or `-1`:** the command's arguments are wrong. Check the command in
  your script.

Some commands return `0` even when part of the work failed:

- `repo-add` and `repo-update` return `0` when Git fails.
- `repo-delete` returns `0` when it can't find a recipe repository.
- `audit` returns `0` when it can't find a recipe, unless a finding reaches the
  `--fail-on` severity.

Check the output of these commands, or follow them with a command that
verifies the result, such as `autopkg repo-list`.

## Related pages

- [Command-line reference](/reference/cli/)
- [`autopkg run`](/reference/cli/autopkg-run/)
- [Schedule recipe runs](/guides/schedule-runs/)
