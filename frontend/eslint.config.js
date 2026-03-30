// @ts-check
import js from '@eslint/js';
import tseslint from '@typescript-eslint/eslint-plugin';
import tsparser from '@typescript-eslint/parser';
import react from 'eslint-plugin-react';
import reactHooks from 'eslint-plugin-react-hooks';
import prettierConfig from 'eslint-config-prettier';

export default [
  // 忽略文件
  {
    ignores: ['dist/**', 'node_modules/**', '*.config.ts', '*.config.js'],
  },

  // 全局设置
  {
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      parser: tsparser,
      parserOptions: {
        project: './tsconfig.json',
        ecmaFeatures: {
          jsx: true,
        },
      },
      globals: {
        browser: true,
        es2020: true,
        node: true,
      },
    },
    settings: {
      react: {
        version: 'detect',
      },
    },
  },

  // 基础 JS 规则
  js.configs.recommended,

  // TypeScript 规则
  {
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
    rules: {
      'no-console': ['warn', { allow: ['warn', 'error'] }],
      'prefer-const': 'warn',
      'no-var': 'error',
      eqeqeq: ['warn', 'always'],
    },
  },

  // Prettier 集成（必须放在最后，关闭与 Prettier 冲突的规则）
  prettierConfig,
];
