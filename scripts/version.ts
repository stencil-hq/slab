// `just version major|minor|patch` — release the next version.
//
// Bumps the three npm package manifests in lockstep, refreshes bun.lock,
// commits `chore: bump version to X.Y.Z`, tags vX.Y.Z, and pushes the branch
// and tag. The tag push triggers .github/workflows/release.yml, which builds,
// runs the tarball e2e, and publishes to npm via trusted publishing.
//
//   bun scripts/version.ts patch [--dry-run]
//
// --dry-run prints the computed version and stops before writing anything.

import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '..');
const PACKAGES = ['clients/web', 'packages/dslab', 'packages/slab'];

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const part = args.find((a) => !a.startsWith('-'));
if (part !== 'major' && part !== 'minor' && part !== 'patch') {
   console.error('usage: just version major|minor|patch [--dry-run]');
   process.exit(1);
}

const run = (cmd: string[]) => {
   console.log(`version: ${cmd.join(' ')}`);
   const res = Bun.spawnSync({ cmd, cwd: ROOT, stdio: ['inherit', 'inherit', 'inherit'] });
   if (res.exitCode !== 0) process.exit(res.exitCode ?? 1);
};

// All three packages release in lockstep; the first manifest is the source of truth.
const manifests = PACKAGES.map((dir) => join(ROOT, dir, 'package.json'));
const current = JSON.parse(readFileSync(manifests[0], 'utf8')).version as string;
const m = current.match(/^(\d+)\.(\d+)\.(\d+)$/);
if (!m) {
   console.error(`version: cannot parse '${current}' in ${manifests[0]}`);
   process.exit(1);
}
let [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
if (part === 'major') [major, minor, patch] = [major + 1, 0, 0];
else if (part === 'minor') [minor, patch] = [minor + 1, 0];
else patch += 1;
const next = `${major}.${minor}.${patch}`;

console.log(`version: ${current} -> ${next} (tag v${next})`);
if (dryRun) process.exit(0);

for (const file of manifests) {
   const pkg = JSON.parse(readFileSync(file, 'utf8'));
   pkg.version = next;
   writeFileSync(file, `${JSON.stringify(pkg, null, 3)}\n`);
}

// bun skips lockfile rewrites when resolution is unchanged, leaving the old
// workspace versions in bun.lock — regenerate it so the lock matches.
rmSync(join(ROOT, 'bun.lock'));
run(['bun', 'install']);
run([
   'git',
   'commit',
   '-m',
   `chore: bump version to ${next}`,
   '--',
   ...PACKAGES.map((dir) => `${dir}/package.json`),
   'bun.lock',
]);
run(['git', 'tag', `v${next}`]);
run(['git', 'push', 'origin', 'HEAD', `v${next}`]);
