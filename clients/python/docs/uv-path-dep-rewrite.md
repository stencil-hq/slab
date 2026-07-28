# uv rewrites absolute path deps to broken relative paths (external issue)

Status: external (astral-sh/uv). This file holds the deterministic repro and
the upstream-ready issue text. The README's "Installing from a path" section
carries the user-facing mitigation.

Observed with uv 0.11.25 on macOS (arm64). Found while dogfooding the
documented external-consumer flow `uv add /work/slab-lang/clients/python`.

## Minimal deterministic repro

Any absolute dependency path whose prefix is a symlink (`/tmp` →
`/private/tmp` on macOS) triggers it; the project location does not matter.

```sh
# a trivial installable package
mkdir -p /tmp/uvrepro/lib/src/uvrepro_lib /tmp/uvrepro/app
cat > /tmp/uvrepro/lib/pyproject.toml <<'EOF'
[project]
name = "uvrepro-lib"
version = "0.1.0"

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
EOF
echo 'VALUE = 42' > /tmp/uvrepro/lib/src/uvrepro_lib/__init__.py

# a consumer that adds it by ABSOLUTE path
cd /tmp/uvrepro/app
uv init --bare
uv add /tmp/uvrepro/lib
tail -2 pyproject.toml
```

Recorded source (uv 0.11.25):

```toml
[tool.uv.sources]
uvrepro-lib = { path = "../../../../tmp/uvrepro/lib" }
```

The relative path is computed lexically between the *resolved* cwd
(`/private/tmp/uvrepro/app`) and the *literal* argument (`/tmp/uvrepro/lib`),
so it climbs four levels — above `/` once re-joined to the unresolved project
directory. Every out-of-cwd invocation now fails:

```sh
cd /Users
uv run --project /tmp/uvrepro/app python -c 'import uvrepro_lib'
# error: Failed to generate package metadata for `app==0.1.0 @ virtual+.`
#   Caused by: cannot normalize a relative path beyond the base directory:
#   /tmp/uvrepro/app/../../../../tmp/uvrepro/lib
```

Running from inside `/tmp/uvrepro/app` still works, which makes the breakage
look like a consumer bug rather than a recorded-path bug.

## Mitigations (verified)

1. After `uv add`, hand-edit `[tool.uv.sources]` back to the absolute path:
   `{ path = "/tmp/uvrepro/lib" }`. uv accepts it and does not rewrite an
   existing entry; `uv run --project` then works from any directory.
2. Or pass fully resolved paths to `uv add` (both sides of the symlink), e.g.
   `cd "$(realpath .)" && uv add "$(realpath /tmp/uvrepro/lib)"` — the
   computed relative path (`../../uvrepro/lib`) is then correct.

## Upstream issue text (ready to file against astral-sh/uv)

> **Title:** `uv add <absolute path>` records a relative `tool.uv.sources`
> path that breaks `uv run --project` when the cwd contains a symlink
>
> **Version:** uv 0.11.25 (macOS arm64, Homebrew); reproduced 2026-07-28.
>
> **Summary:** `uv add /abs/path/to/dep` silently rewrites the absolute path
> to a relative one under `[tool.uv.sources]`. The relative path is computed
> lexically between the symlink-resolved current directory and the unresolved
> argument, so on macOS (where `/tmp` is a symlink to `/private/tmp`) a
> project under `/tmp` records a path such as `../../../../tmp/uvrepro/lib`
> — four components up from a directory only three below `/`. Any later
> `uv run --project <dir>` from another cwd fails with:
>
> ```
> error: Failed to generate package metadata for `app==0.1.0 @ virtual+.`
>   Caused by: cannot normalize a relative path beyond the base directory:
>   /tmp/uvrepro/app/../../../../tmp/uvrepro/lib
> ```
>
> Running from inside the project directory still works, which hides the
> corruption until the project is driven from elsewhere (CI, `--project`
> automation, editor tooling).
>
> **Repro (deterministic):**
>
> ```sh
> mkdir -p /tmp/uvrepro/lib/src/uvrepro_lib /tmp/uvrepro/app
> printf '[project]\nname = "uvrepro-lib"\nversion = "0.1.0"\n\n[build-system]\nrequires = ["hatchling"]\nbuild-backend = "hatchling.build"\n' > /tmp/uvrepro/lib/pyproject.toml
> printf 'VALUE = 42\n' > /tmp/uvrepro/lib/src/uvrepro_lib/__init__.py
> cd /tmp/uvrepro/app && uv init --bare && uv add /tmp/uvrepro/lib
> tail -2 pyproject.toml   # { path = "../../../../tmp/uvrepro/lib" }
> cd /Users && uv run --project /tmp/uvrepro/app python -c 'import uvrepro_lib'
> ```
>
> **Expected:** either keep the absolute path the user typed (they asked for
> an absolute dependency), or compute the relative path between consistently
> resolved (or consistently unresolved) endpoints so the recorded path
> round-trips.
>
> **Actual:** the recorded relative path only resolves from directories whose
> symlink expansion happens to match the one used at `uv add` time.
>
> **Workarounds:** hand-edit `[tool.uv.sources]` back to the absolute path
> (uv preserves an existing absolute entry), or invoke `uv add` with
> `realpath`-resolved project and dependency paths.
