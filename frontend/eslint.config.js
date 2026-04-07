// @ts-check
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import js from '@eslint/js';
import tseslint from '@typescript-eslint/eslint-plugin';
import tsparser from '@typescript-eslint/parser';
import react from 'eslint-plugin-react';
import reactHooks from 'eslint-plugin-react-hooks';
import prettierConfig from 'eslint-config-prettier';

const configDir = path.dirname(fileURLToPath(import.meta.url));
const srcFiles = ['src/**/*.{ts,tsx}'];

export default [
  // 忽略文件
  {
    ignores: [
      'dist/**',
      'build/**',
      'coverage/**',
      'node_modules/**',
      '*.min.js',
      '*.config.ts',
      '*.config.js',
    ],
  },

  // 把规则显式限定到 src 源码，避免 flat config 在 CLI 传入目录时把整个树判成 ignored。
  {
    files: srcFiles,
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      parser: tsparser,
      parserOptions: {
        project: './tsconfig.json',
        tsconfigRootDir: configDir,
        ecmaFeatures: {
          jsx: true,
        },
      },
    },
    settings: {
      react: {
        version: 'detect',
      },
    },
  },

  // 基础 JS 规则
  {
    ...js.configs.recommended,
    files: srcFiles,
    rules: {
      ...js.configs.recommended.rules,
      // TypeScript 已负责全局名和类型检查，这里关闭 no-undef 避免和 TS 语义重复冲突。
      'no-undef': 'off',
    },
  },

  // TypeScript 规则
  {
    files: srcFiles,
    plugins: {
      '@typescript-eslint': tseslint,
    },
    rules: {
      ...tseslint.configs['recommended-requiring-type-checking'].rules,
      '@typescript-eslint/no-unused-vars': [
        'warn',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/no-explicit-any': 'warn',
      '@typescript-eslint/explicit-function-return-type': 'off',
      '@typescript-eslint/explicit-module-boundary-types': 'off',
      '@typescript-eslint/no-non-null-assertion': 'warn',
      '@typescript-eslint/no-floating-promises': 'warn',
      '@typescript-eslint/await-thenable': 'warn',
      '@typescript-eslint/no-misused-promises': 'warn',
    },
  },

  // React 规则
  {
    files: srcFiles,
    plugins: {
      react,
      'react-hooks': reactHooks,
    },
    rules: {
      ...react.configs.recommended.rules,
      ...react.configs['jsx-runtime'].rules,
      ...reactHooks.configs.recommended.rules,
      'react/prop-types': 'off',
      'react/no-unescaped-entities': 'warn',
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
    },
  },

  // 通用规则
  {
    files: srcFiles,
    rules: {
      'no-console': ['warn', { allow: ['warn', 'error'] }],
      'prefer-const': 'warn',
      'no-var': 'error',
      eqeqeq: ['warn', 'always'],
    },
  },

  // Prettier 集成（必须放在最后，关闭与 Prettier 冲突的规则）
  {
    ...prettierConfig,
    files: srcFiles,
  },
];
