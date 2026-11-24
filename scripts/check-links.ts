#!/usr/bin/env bun
import { readdir, readFile, stat } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';

const root = process.cwd();
const distDir = join(root, 'dist');

const config = await readFile(join(root, 'astro.config.mjs'), 'utf8');
const base = /(?:const\s+base\s*=\s*|base:\s*)['"]([^'"]+)['"]/.exec(config)?.[1] ?? '/';

type Finding = { source: string; href: string };

async function walk(dir: string): Promise<string[]> {
	const entries = await readdir(dir, { withFileTypes: true });
	const files: string[] = [];
	for (const entry of entries) {
		const full = join(dir, entry.name);
		if (entry.isDirectory()) files.push(...(await walk(full)));
		else if (entry.name.endsWith('.html')) files.push(full);
	}
	return files.sort();
}

async function exists(path: string): Promise<boolean> {
	try {
		await stat(path);
		return true;
	} catch {
		return false;
	}
}

async function resolveTarget(sourceFile: string, href: string): Promise<string | null> {
	const clean = href.split('#')[0]?.split('?')[0] ?? '';
	if (!clean) return null;
	if (/^(?:https?:|mailto:|tel:|data:|javascript:)/i.test(clean)) return null;

	if (clean.startsWith('/')) {
		const withoutBase =
			base !== '/' && (clean === base || clean.startsWith(`${base}/`))
				? clean.slice(base.length) || '/'
				: clean;
		const target = join(distDir, withoutBase);
		const candidates = [
			target,
			join(target, 'index.html'),
			`${target}.html`,
		];
		for (const candidate of candidates) {
			if (await exists(candidate)) return candidate;
		}
		return target;
	}

	const resolved = resolve(dirname(sourceFile), clean);
	const candidates = [resolved, join(resolved, 'index.html'), `${resolved}.html`];
	for (const candidate of candidates) {
		if (await exists(candidate)) return candidate;
	}
	return resolved;
}

if (!(await exists(distDir))) {
	console.error('dist/ does not exist. Run bun run build first.');
	process.exit(1);
}

const files = await walk(distDir);
const findings: Finding[] = [];
let checked = 0;

for (const file of files) {
	const html = await readFile(file, 'utf8');
	const attrs = html.matchAll(/\b(?:href|src)="([^"]+)"/g);
	for (const match of attrs) {
		const value = match[1] ?? '';
		if (!value) continue;
		const target = await resolveTarget(file, value);
		if (target === null) continue;
		checked++;
		if (!(await exists(target))) {
			findings.push({ source: relative(root, file), href: value });
		}
	}
}

if (findings.length > 0) {
	for (const finding of findings) {
		console.error(`${finding.source}  ${finding.href}`);
	}
	console.error(`\n${findings.length} broken internal link(s); ${checked} checked.`);
	process.exit(1);
}

console.log(`All ${checked} internal links resolve.`);
