(() => {
	const classify = (line) => {
		const code = line.querySelector('.code');
		if (!code) return;

		const text = code.textContent ?? '';
		const trimmed = text.trimStart();
		const isCommand = trimmed.startsWith('$');

		line.classList.toggle('console-command', isCommand);
		if (isCommand) {
			line.classList.remove('console-error', 'console-warning', 'console-success');
			return;
		}

		line.classList.toggle(
			'console-error',
			/\berror(?:\[|:)|panicked at|test result: FAILED|FAILED\b/i.test(text),
		);
		line.classList.toggle('console-warning', /\bwarning(?:\[|:)/i.test(text));
		line.classList.toggle(
			'console-success',
			/test result: ok|^test\s+.*\.\.\.\s+ok\b|^Finished\b/.test(trimmed),
		);
	};

	const color = () => {
		document
			.querySelectorAll('.expressive-code .frame.is-terminal pre .ec-line')
			.forEach(classify);
	};

	if (document.readyState === 'loading') {
		document.addEventListener('DOMContentLoaded', color, { once: true });
	} else {
		color();
	}

	document.addEventListener('astro:page-load', color);
})();
