import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

interface WebChatConfigProfile {
  name?: string;
  baseUrl?: string;
  apiKey?: string | null;
  models?: string[];
}

interface WebChatConfig {
  activeProfile?: string;
  activeModel?: string;
  profiles?: WebChatConfigProfile[];
}

interface WebChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

const APP_HOME_OVERRIDE_ENV = 'ASTRCODE_HOME_DIR';

function webChatPlugin(): Plugin {
  return {
    name: 'astrcode-web-chat',
    configureServer(server) {
      server.middlewares.use('/api/web-chat', async (req, res) => {
        if (req.method !== 'POST') {
          res.statusCode = 405;
          res.setHeader('Content-Type', 'application/json; charset=utf-8');
          res.end(JSON.stringify({ error: 'Method Not Allowed' }));
          return;
        }

        try {
          const body = await readJsonBody(req);
          const turnId = typeof body.turnId === 'string' ? body.turnId : `web-${Date.now()}`;
          const messages = normalizeMessages(body.messages);
          const profile = await loadActiveProfile();
          const apiKey = resolveApiKey(profile.apiKey);
          const model = resolveModel(body.model, profile.activeModel, profile.models);
          const endpoint = `${(profile.baseUrl || 'https://api.deepseek.com').replace(/\/+$/, '')}/chat/completions`;

          const upstream = await fetch(endpoint, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              Authorization: `Bearer ${apiKey}`,
            },
            body: JSON.stringify({
              model,
              stream: true,
              messages,
            }),
          });

          if (!upstream.ok || !upstream.body) {
            const text = await upstream.text();
            throw new Error(`上游模型请求失败 ${upstream.status}: ${text || upstream.statusText}`);
          }

          res.statusCode = 200;
          res.setHeader('Content-Type', 'application/x-ndjson; charset=utf-8');
          res.setHeader('Cache-Control', 'no-cache, no-transform');
          res.setHeader('Connection', 'keep-alive');

          writeNdjson(res, {
            event: 'phaseChanged',
            data: { phase: 'thinking', turnId },
          });

          const reader = upstream.body.getReader();
          const decoder = new TextDecoder();
          let buffer = '';
          let streamingStarted = false;

          while (true) {
            const { value, done } = await reader.read();
            if (done) {
              break;
            }

            buffer += decoder.decode(value, { stream: true });
            const segments = buffer.split(/\r?\n/);
            buffer = segments.pop() ?? '';

            for (const line of segments) {
              const parsed = parseSseDataLine(line);
              if (!parsed) {
                continue;
              }
              if (parsed === '[DONE]') {
                writeNdjson(res, { event: 'turnDone', data: { turnId } });
                writeNdjson(res, {
                  event: 'phaseChanged',
                  data: { phase: 'idle', turnId },
                });
                res.end();
                return;
              }

              const delta = parsed.choices?.[0]?.delta?.content;
              if (!delta) {
                continue;
              }

              if (!streamingStarted) {
                streamingStarted = true;
                writeNdjson(res, {
                  event: 'phaseChanged',
                  data: { phase: 'streaming', turnId },
                });
              }

              writeNdjson(res, {
                event: 'modelDelta',
                data: { turnId, delta },
              });
            }
          }

          writeNdjson(res, { event: 'turnDone', data: { turnId } });
          writeNdjson(res, {
            event: 'phaseChanged',
            data: { phase: 'idle', turnId },
          });
          res.end();
        } catch (error) {
          res.statusCode = 500;
          res.setHeader('Content-Type', 'application/json; charset=utf-8');
          res.end(
            JSON.stringify({
              error: error instanceof Error ? error.message : String(error),
            }),
          );
        }
      });
    },
  };
}

async function readJsonBody(req: NodeJS.ReadableStream): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }

  const raw = Buffer.concat(chunks).toString('utf8');
  return raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
}

function normalizeMessages(input: unknown): WebChatMessage[] {
  if (!Array.isArray(input)) {
    throw new Error('messages 必须是数组');
  }

  const messages = input
    .map((item) => {
      if (!item || typeof item !== 'object') {
        return null;
      }

      const role = (item as { role?: unknown }).role;
      const content = (item as { content?: unknown }).content;
      if (
        (role === 'system' || role === 'user' || role === 'assistant') &&
        typeof content === 'string' &&
        content.trim()
      ) {
        return { role, content } satisfies WebChatMessage;
      }

      return null;
    })
    .filter((item): item is WebChatMessage => item !== null);

  if (messages.length === 0) {
    throw new Error('messages 不能为空');
  }

  return messages;
}

async function loadActiveProfile(): Promise<Required<Pick<WebChatConfigProfile, 'baseUrl' | 'apiKey' | 'models'>> & { activeModel: string | null }> {
  const configPath = path.join(resolveAstrcodeHomeDir(), '.astrcode', 'config.json');
  const raw = await fs.readFile(configPath, 'utf8');
  const config = JSON.parse(raw) as WebChatConfig;
  const profiles = Array.isArray(config.profiles) ? config.profiles : [];
  const activeProfileName = config.activeProfile || 'default';
  const profile = profiles.find((item) => item.name === activeProfileName) || profiles[0];

  if (!profile) {
    throw new Error(`未在 ${configPath} 找到可用 profile`);
  }

  return {
    baseUrl: profile.baseUrl || 'https://api.deepseek.com',
    apiKey: profile.apiKey ?? null,
    activeModel: config.activeModel || null,
    models: Array.isArray(profile.models) ? profile.models : [],
  };
}

function resolveApiKey(apiKey: string | null): string {
  const value = apiKey?.trim();
  if (!value) {
    throw new Error('AstrCode 配置中的 apiKey 为空');
  }

  const looksLikeEnvName = /^[A-Z0-9_]+$/.test(value) && value.includes('_');
  if (looksLikeEnvName) {
    const envValue = process.env[value]?.trim();
    if (!envValue) {
      throw new Error(`环境变量 ${value} 未设置`);
    }
    return envValue;
  }

  return value;
}

function resolveAstrcodeHomeDir(): string {
  const overriddenHome = process.env[APP_HOME_OVERRIDE_ENV]?.trim();
  if (overriddenHome) {
    return overriddenHome;
  }

  return os.homedir();
}

function resolveModel(requested: unknown, activeModel: string | null, models: string[]): string {
  if (typeof requested === 'string' && requested.trim()) {
    return requested.trim();
  }

  if (activeModel && activeModel.trim()) {
    return activeModel.trim();
  }

  if (models.length > 0) {
    return models[0];
  }

  return 'deepseek-chat';
}

function parseSseDataLine(line: string): '[DONE]' | { choices?: Array<{ delta?: { content?: string } }> } | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith('data:')) {
    return null;
  }

  const payload = trimmed.slice(5).trim();
  if (!payload) {
    return null;
  }
  if (payload === '[DONE]') {
    return '[DONE]';
  }

  return JSON.parse(payload) as { choices?: Array<{ delta?: { content?: string } }> };
}

function writeNdjson(res: { write: (chunk: string) => void }, payload: unknown): void {
  res.write(`${JSON.stringify(payload)}\n`);
}

export default defineConfig({
  plugins: [react(), webChatPlugin()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
});
