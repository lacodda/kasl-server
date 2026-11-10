import js from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'
import dowel from 'dowel-ui/eslint'

export default tseslint.config(
  { ignores: ['dist'] },
  js.configs.recommended,
  tseslint.configs.recommended,
  // The `flat` variant; the top-level one is still in the legacy shape.
  reactHooks.configs.flat['recommended-latest'],
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
  },
  {
    // Dates and numbers in the app's language, never the machine's: a
    // formatter given no locale put Russian weekdays in an English page once
    // (dowel 0.31). The rest of dowel's set arrives with the next upgrade.
    files: ['**/*.{ts,tsx}'],
    plugins: { dowel },
    rules: { 'dowel/no-implicit-locale': 'error' },
  },
  {
    files: ['*.config.{js,ts}'],
    languageOptions: { globals: globals.node },
  },
)
