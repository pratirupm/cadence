#!/usr/bin/env bun
import { spawnSync } from 'node:child_process';
import { mkdir, rm, stat } from 'node:fs/promises';

// Ship the reference app as a companion archive, not as part of the manuscript.
await mkdir('public/reference', { recursive: true });
await rm('public/reference/cadence-app.tar.gz', { force: true });

const archive = 'public/reference/cadence-app.tar.gz';
const result = spawnSync(
	'tar',
	[
		'-czf',
		archive,
		'--exclude=cadence-app/target',
		'--exclude=cadence-app/cadence.db',
		'--exclude=cadence-app/cadence.db-*',
		'--exclude=cadence-app/.env',
		'cadence-app',
	],
	{ stdio: 'inherit' },
);

if (result.error || result.status !== 0) {
	throw new Error('failed to package cadence-app');
}

const { size } = await stat(archive);
console.log(`packaged ${archive} (${Math.round(size / 1024)} KiB)`);
