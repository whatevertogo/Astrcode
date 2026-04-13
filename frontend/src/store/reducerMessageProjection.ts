import { appendToolDeltaMetadata, mergeToolMetadata } from '../lib/toolDisplay';
import type { Action, AppState, Message, Session } from '../types';
import { buildSubRunThreadTree, patchSubRunThreadTreeMessages } from '../lib/subRunView';
import { uuid } from '../utils/uuid';
import {
  findAssistantMessageIndex,
  findPromptMetricsMessageIndex,
  findToolCallMessageIndex,
  findUserMessageIndex,
  mapSession,
  moveUpdatedMessageToTail,
  upsertAssistantTurnMessage,
} from './reducerHelpers';

function withUpdatedMessages(
  session: Session,
  messages: Message[],
  options?: { forceRebuild?: boolean }
): Session {
  if (!options?.forceRebuild) {
    const patchedTree = patchSubRunThreadTreeMessages(session.subRunThreadTree, messages);
    if (patchedTree) {
      return {
        ...session,
        messages,
        subRunThreadTree: patchedTree,
      };
    }
  }

  return {
    ...session,
    messages,
    subRunThreadTree: buildSubRunThreadTree(messages),
  };
}

function isStructuralMessage(message: Message): boolean {
  if (
    message.kind === 'subRunStart' ||
    message.kind === 'subRunFinish' ||
    message.kind === 'childSessionNotification'
  ) {
    return true;
  }

  // Why: spawn 工具会作为 lifecycle 缺失时的 sub-run fallback 事实源，
  // 它的新增/更新必须触发结构重建，避免父视图树漏子执行。
  return message.kind === 'toolCall' && message.toolName === 'spawn';
}

export function handleProjectedMessageAction(state: AppState, action: Action): AppState | null {
  switch (action.type) {
    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        let title = session.title;
        if (
          action.message.kind === 'user' &&
          session.messages.filter((message) => message.kind === 'user').length === 0
        ) {
          title = action.message.text.slice(0, 20) || '新会话';
        }
        const nextMessages = [...session.messages, action.message];
        const withProjection = withUpdatedMessages(session, nextMessages, {
          forceRebuild: isStructuralMessage(action.message),
        });
        return title === session.title
          ? withProjection
          : {
              ...withProjection,
              title,
            };
      });

    case 'UPSERT_USER_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findUserMessageIndex(session.messages, action.turnId);
        const existingUserMessage =
          targetIndex >= 0 && session.messages[targetIndex]?.kind === 'user'
            ? session.messages[targetIndex]
            : null;
        const userMessage = {
          id: existingUserMessage?.id ?? uuid(),
          kind: 'user' as const,
          turnId: action.turnId,
          agentId: action.agentId ?? existingUserMessage?.agentId,
          parentTurnId: action.parentTurnId ?? existingUserMessage?.parentTurnId,
          agentProfile: action.agentProfile ?? existingUserMessage?.agentProfile,
          subRunId: action.subRunId ?? existingUserMessage?.subRunId,
          executionId: action.executionId ?? existingUserMessage?.executionId,
          invocationKind: action.invocationKind ?? existingUserMessage?.invocationKind,
          storageMode: action.storageMode ?? existingUserMessage?.storageMode,
          childSessionId: action.childSessionId ?? existingUserMessage?.childSessionId,
          text: action.content,
          timestamp: existingUserMessage?.timestamp ?? Date.now(),
        };

        let title = session.title;
        if (session.messages.filter((message) => message.kind === 'user').length === 0) {
          title = action.content.slice(0, 20) || '新会话';
        }

        if (targetIndex < 0) {
          const next = withUpdatedMessages(session, [...session.messages, userMessage], {
            // 新增消息通常无法增量 patch（旧 tree 不存在该 message id），
            // 这里直接重建避免无意义的 patch 扫描。
            forceRebuild: true,
          });
          return title === session.title
            ? next
            : {
                ...next,
                title,
              };
        }

        const next = withUpdatedMessages(
          session,
          moveUpdatedMessageToTail(session.messages, targetIndex, userMessage)
        );
        return title === session.title
          ? next
          : {
              ...next,
              title,
            };
      });

    case 'APPEND_DELTA':
      return mapSession(state, action.sessionId, (session) =>
        withUpdatedMessages(
          session,
          upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              executionId: action.executionId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: action.delta,
              reasoningText: '',
              streaming: true,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              executionId: action.executionId ?? message.executionId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              text: message.text + action.delta,
              streaming: true,
            })
          )
        )
      );

    case 'APPEND_REASONING_DELTA':
      return mapSession(state, action.sessionId, (session) =>
        withUpdatedMessages(
          session,
          upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              executionId: action.executionId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: '',
              reasoningText: action.delta,
              streaming: true,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              executionId: action.executionId ?? message.executionId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              reasoningText: `${message.reasoningText ?? ''}${action.delta}`,
              streaming: true,
            })
          )
        )
      );

    case 'FINALIZE_ASSISTANT':
      return mapSession(state, action.sessionId, (session) =>
        withUpdatedMessages(
          session,
          upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              executionId: action.executionId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: action.content,
              reasoningText: action.reasoningText,
              streaming: false,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              executionId: action.executionId ?? message.executionId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              text: action.content,
              reasoningText: action.reasoningText ?? message.reasoningText,
              streaming: false,
            })
          )
        )
      );

    case 'END_STREAMING':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findAssistantMessageIndex(session.messages, action.turnId);
        if (targetIndex < 0) {
          return session;
        }

        const target = session.messages[targetIndex];
        if (target.kind !== 'assistant') {
          return session;
        }

        return withUpdatedMessages(
          session,
          moveUpdatedMessageToTail(session.messages, targetIndex, {
            ...target,
            streaming: false,
          })
        );
      });

    case 'APPEND_TOOL_CALL_DELTA':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findToolCallMessageIndex(
          session.messages,
          action.toolCallId,
          action.toolName,
          action.turnId,
          false
        );

        if (targetIndex < 0) {
          return withUpdatedMessages(
            session,
            [
              ...session.messages,
              {
                id: uuid(),
                kind: 'toolCall',
                turnId: action.turnId,
                agentId: action.agentId,
                parentTurnId: action.parentTurnId,
                agentProfile: action.agentProfile,
                subRunId: action.subRunId,
                executionId: action.executionId,
                invocationKind: action.invocationKind,
                storageMode: action.storageMode,
                childSessionId: action.childSessionId,
                toolCallId: action.toolCallId,
                toolName: action.toolName,
                status: 'running',
                args: null,
                output: action.delta,
                metadata: appendToolDeltaMetadata(
                  undefined,
                  action.toolName,
                  null,
                  action.stream,
                  action.delta
                ),
                timestamp: Date.now(),
              },
            ],
            {
              forceRebuild: action.toolName === 'spawn',
            }
          );
        }

        return withUpdatedMessages(
          session,
          session.messages.map((message, index) => {
            if (index !== targetIndex || message.kind !== 'toolCall') {
              return message;
            }
            return {
              ...message,
              turnId: action.turnId ?? message.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              executionId: action.executionId ?? message.executionId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              toolCallId: action.toolCallId,
              toolName: action.toolName,
              output: `${message.output ?? ''}${action.delta}`,
              metadata: appendToolDeltaMetadata(
                message.metadata,
                action.toolName,
                message.args,
                action.stream,
                action.delta
              ),
            };
          }),
          {
            forceRebuild: action.toolName === 'spawn',
          }
        );
      });

    case 'UPDATE_TOOL_CALL':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findToolCallMessageIndex(
          session.messages,
          action.toolCallId,
          action.toolName,
          action.turnId,
          true
        );

        if (targetIndex < 0) {
          return withUpdatedMessages(
            session,
            [
              ...session.messages,
              {
                id: uuid(),
                kind: 'toolCall',
                turnId: action.turnId,
                agentId: action.agentId,
                parentTurnId: action.parentTurnId,
                agentProfile: action.agentProfile,
                subRunId: action.subRunId,
                executionId: action.executionId,
                invocationKind: action.invocationKind,
                storageMode: action.storageMode,
                childSessionId: action.childSessionId,
                toolCallId: action.toolCallId,
                toolName: action.toolName,
                status: action.status,
                args: null,
                output: action.output,
                error: action.error,
                metadata: action.metadata,
                durationMs: action.durationMs,
                truncated: action.truncated,
                timestamp: Date.now(),
              },
            ],
            {
              forceRebuild: action.toolName === 'spawn',
            }
          );
        }

        return withUpdatedMessages(
          session,
          session.messages.map((message, index) => {
            if (index !== targetIndex || message.kind !== 'toolCall') {
              return message;
            }
            const isShellTool = message.toolName === 'shell' || action.toolName === 'shell';
            return {
              ...message,
              turnId: action.turnId ?? message.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              executionId: action.executionId ?? message.executionId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              toolCallId: action.toolCallId,
              toolName: action.toolName,
              status: action.status,
              output: isShellTool && message.output ? message.output : action.output,
              error: action.error,
              metadata: mergeToolMetadata(message.metadata, action.metadata),
              durationMs: action.durationMs,
              truncated: action.truncated,
            };
          }),
          {
            forceRebuild: action.toolName === 'spawn',
          }
        );
      });

    case 'UPSERT_PROMPT_METRICS':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findPromptMetricsMessageIndex(
          session.messages,
          action.stepIndex,
          action.turnId
        );
        const existingPromptMetrics =
          targetIndex >= 0 && session.messages[targetIndex]?.kind === 'promptMetrics'
            ? session.messages[targetIndex]
            : null;
        const nextMessage = {
          id: existingPromptMetrics?.id ?? uuid(),
          kind: 'promptMetrics' as const,
          turnId: action.turnId ?? null,
          agentId: action.agentId ?? existingPromptMetrics?.agentId,
          parentTurnId: action.parentTurnId ?? existingPromptMetrics?.parentTurnId,
          agentProfile: action.agentProfile ?? existingPromptMetrics?.agentProfile,
          subRunId: action.subRunId ?? existingPromptMetrics?.subRunId,
          executionId: action.executionId ?? existingPromptMetrics?.executionId,
          invocationKind: action.invocationKind ?? existingPromptMetrics?.invocationKind,
          storageMode: action.storageMode ?? existingPromptMetrics?.storageMode,
          childSessionId: action.childSessionId ?? existingPromptMetrics?.childSessionId,
          stepIndex: action.stepIndex,
          estimatedTokens: action.estimatedTokens,
          contextWindow: action.contextWindow,
          effectiveWindow: action.effectiveWindow,
          thresholdTokens: action.thresholdTokens,
          truncatedToolResults: action.truncatedToolResults,
          providerInputTokens: action.providerInputTokens,
          providerOutputTokens: action.providerOutputTokens,
          cacheCreationInputTokens: action.cacheCreationInputTokens,
          cacheReadInputTokens: action.cacheReadInputTokens,
          providerCacheMetricsSupported: action.providerCacheMetricsSupported,
          promptCacheReuseHits: action.promptCacheReuseHits,
          promptCacheReuseMisses: action.promptCacheReuseMisses,
          timestamp: existingPromptMetrics?.timestamp ?? Date.now(),
        };

        if (targetIndex < 0) {
          return withUpdatedMessages(session, [...session.messages, nextMessage], {
            forceRebuild: true,
          });
        }

        return withUpdatedMessages(
          session,
          moveUpdatedMessageToTail(session.messages, targetIndex, nextMessage)
        );
      });

    default:
      return null;
  }
}
