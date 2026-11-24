export default [
	{
		ignores: ['build', '.svelte-kit', 'dist']
	},
	{
		files: ['**/*.ts', '**/*.svelte'],
		languageOptions: {
			ecmaVersion: 2020,
			sourceType: 'module',
			globals: {
				browser: 'readonly',
				node: 'readonly'
			}
		}
	}
];
