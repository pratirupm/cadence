#!/usr/bin/env bun
/**
 * Prefix root-relative links in the built HTML with the Astro `base`.
 *
 * Starlight prefixes its own navigation, but hand-written Markdown links and
 * hero actions stay root-relative. GitHub Pages serves a project site under
 * `/<repo>/`, so those links need the same prefix.
 */
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const root = process.cwd();
const distDir = join(root, 'dist');

const config = await readFile(join(root, 'astro.config.mjs'), 'utf8');
const base = /(?:const\s+base\s*=\s*|base:\s*)['"]([^'"]+)['"]/.exec(config)?.[1] ?? '/';

if (base === '/') {
	console.log('No base configured; nothing to prefix.');
	process.exit(0);
}

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

let changed = 0;
let links = 0;

for (const file of await walk(distDir)) {
	const html = await readFile(file, 'utf8');
	const withBase = html.replace(/\b(href|src)="(\/(?!\/)[^"]*)"/g, (whole, attr, path) => {
		if (path === base || path.startsWith(`${base}/`)) return whole;
		links++;
		return `${attr}="${base}${path}"`;
	});

	if (withBase !== html) {
		await writeFile(file, withBase);
		changed++;
	}
}

console.log(`Prefixed ${links} root-relative link(s) in ${changed} HTML file(s).`);
