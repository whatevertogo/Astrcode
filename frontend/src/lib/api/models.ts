//! # Model API Endpoints
//!
//! Model discovery and connection testing.

import type { CurrentModelInfo, ModelOption, TestResult } from '../../types';
import { requestJson } from './client';

export async function getCurrentModel(): Promise<CurrentModelInfo> {
  return requestJson<CurrentModelInfo>('/api/models/current');
}

export async function listAvailableModels(): Promise<ModelOption[]> {
  return requestJson<ModelOption[]>('/api/models');
}

export async function testConnection(profileName: string, model: string): Promise<TestResult> {
  return requestJson<TestResult>('/api/models/test', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ profileName, model }),
  });
}
