# Engine compatibility status

The engine executes validated built-in recipe steps in order. Cache creation is
optional at the engine API and requires `CACHE_DIR`. The CLI resolves existing
preferences and the Python-compatible default cache location; comparison tests
supply isolated caches.
Preflight runs before directory creation. Cache boundary checks resolve existing
symlinks and preserve the reference's normalized path spelling in the environment.

`run_recipe_detailed` returns completed receipt entries and the current environment
on failure. `run_recipe` preserves the simpler string-error interface. Failure
receipts end with a `RecipeError` entry. The initial receipt excludes `GITHUB_TOKEN`,
while the runtime environment retains its recipe value. GitHub authentication
uses a separate immutable preference context, not recipe inputs or `--key` values.

The shared `autopkg-value` model preserves integers, real numbers, booleans,
strings, arrays, dictionaries, dates, binary data, and nulls. JSON and YAML retain
nulls. Receipt and report plist serialization maps nested nulls to empty strings,
matching the pinned Python `plist_serializer`; standalone output omits top-level
null fields. Serialization never changes the execution environment. A CLI integration test
compares typed binary-plist and YAML recipes, processor arguments and outputs,
and persisted receipts with values frozen from the pinned Python reference.

The YAML reader preserves scalar tags and quoting, YAML 1.1 booleans/integers,
binary data, timestamps, aliases, and merge keys. AutoPkg deliberately disables
implicit floats: bare `2.3` remains a string; `!!float 2.3` is a real number.
Quoted timestamps remain strings. Custom tags, recursive aliases, excessive
nesting, and unrepresentable integer ranges reject explicitly.

Trust verification supports local overrides with complete built-in parent chains.
The caller supplies override and repository directories. Package-script trust
hashes Git-tracked files when a repository is present, retains the reference's
warning for newly added scripts, and rejects changed or missing trusted scripts.
Custom processors reject. Preference files support plist and JSON; native macOS
preference-domain lookup belongs to the platform integration.
