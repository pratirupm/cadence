// A small grammar for terminal transcripts. Command lines get shell-like
// highlighting; output lines stay plain so they are not mistaken for syntax.
export default {
	name: 'book-console',
	scopeName: 'source.book-console',
	repository: {},
	patterns: [
		{
			begin: '^(\\$)\\s+',
			beginCaptures: {
				1: { name: 'punctuation.definition.prompt.book-console' },
			},
			end: '$',
			patterns: [
				{
					match: '\\b(?:cargo|rustc|git|cd|echo|printf|mkdir|cp|mv|rm|du|ls|cat|sed|grep|tail|head|chmod)\\b',
					name: 'support.function.book-console',
				},
				{
					match: '--?[A-Za-z0-9][A-Za-z0-9-]*',
					name: 'constant.other.book-console',
				},
				{
					match: '"(?:[^"\\\\]|\\\\.)*"|\'(?:[^\'\\\\]|\\\\.)*\'',
					name: 'string.quoted.book-console',
				},
				{
					match: '\\b\\d+\\b',
					name: 'constant.numeric.book-console',
				},
			],
		},
		{
			match: 'https?://\\S+',
			name: 'markup.underline.link.book-console',
		},
	],
};
