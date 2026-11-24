(() => {
	const fix = () => {
		document
			.querySelectorAll('input.pagefind-ui__search-input')
			.forEach((input) => {
				if (!input.getAttribute('aria-label')) {
					input.setAttribute('aria-label', 'Search the book');
				}
			});

		document.querySelectorAll('.expressive-code .copy button').forEach((button) => {
			if (!button.getAttribute('aria-label')) {
				button.setAttribute(
					'aria-label',
					button.getAttribute('title') || 'Copy code',
				);
			}
		});

		const toc = document.getElementById('starlight__on-this-page');
		if (toc && toc.tagName === 'H2') {
			const label = document.createElement('p');
			label.id = toc.id;
			label.className = `${toc.className} toc-title`;
			label.textContent = toc.textContent;
			toc.replaceWith(label);
		}

		document.querySelectorAll('article.card > p.title').forEach((title) => {
			const heading = document.createElement('h2');
			heading.className = title.className;
			heading.innerHTML = title.innerHTML;
			title.replaceWith(heading);
		});
	};

	if (document.readyState === 'loading') {
		document.addEventListener('DOMContentLoaded', fix, { once: true });
	} else {
		fix();
	}

	document.addEventListener('astro:page-load', fix);

	const observer = new MutationObserver(fix);
	observer.observe(document.body, { childList: true, subtree: true });
})();
