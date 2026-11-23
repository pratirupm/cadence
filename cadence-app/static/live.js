(() => {
	const source = new EventSource("/events");
	let timer;

	const refresh = () => {
		clearTimeout(timer);
		timer = setTimeout(() => window.location.reload(), 150);
	};

	for (const event of ["task.created", "task.updated", "task.deleted"]) {
		source.addEventListener(event, refresh);
	}

	source.onerror = () => source.close();
})();
