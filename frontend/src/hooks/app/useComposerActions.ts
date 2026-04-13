import { useCallback, type Dispatch, type MutableRefObject } from 'react';
import { forgetProject, rememberProject } from '../../lib/knownProjects';
import { uuid } from '../../utils/uuid';
import { parseRuntimeSlashCommand } from '../../lib/slashCommands';
import type {
  Action,
  DeleteProjectResult,
  ExecutionControl,
  Phase,
  Project,
  SessionMeta,
} from '../../types';
import { logger } from '../../lib/logger';

export interface ConfirmDialogState {
  title: string;
  message: string;
  danger?: boolean;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm: () => void | Promise<void>;
}

interface PromptSubmission {
  turnId: string;
  sessionId: string;
  branchedFromSessionId?: string;
}

interface UseComposerActionsOptions {
  activeProject: Project | null;
  projects: Project[];
  dispatch: Dispatch<Action>;
  phaseRef: MutableRefObject<Phase>;
  activeSessionIdRef: MutableRefObject<string | null>;
  pendingSubmitSessionRef: MutableRefObject<string[]>;
  turnSessionMapRef: MutableRefObject<Record<string, string>>;
  releasePendingSubmitSession: (sessionId: string) => void;
  setConfirmDialog: React.Dispatch<React.SetStateAction<ConfirmDialogState | null>>;
  refreshSessions: (options?: { preferredSessionId?: string | null }) => Promise<void>;
  createSession: (workingDir: string) => Promise<SessionMeta>;
  submitPrompt: (
    sessionId: string,
    text: string,
    control?: ExecutionControl
  ) => Promise<PromptSubmission>;
  interrupt: (sessionId: string) => Promise<void>;
  compactSession: (
    sessionId: string,
    control?: ExecutionControl
  ) => Promise<{ accepted: boolean; deferred: boolean; message: string }>;
  deleteSession: (sessionId: string) => Promise<void>;
  deleteProject: (workingDir: string) => Promise<DeleteProjectResult>;
}

function appendLocalError(dispatch: Dispatch<Action>, sessionId: string, message: string) {
  dispatch({
    type: 'ADD_MESSAGE',
    sessionId,
    message: {
      id: uuid(),
      kind: 'assistant',
      text: `错误：${message}`,
      reasoningText: '',
      streaming: false,
      timestamp: Date.now(),
    },
  });
}

function appendLocalNotice(dispatch: Dispatch<Action>, sessionId: string, message: string) {
  dispatch({
    type: 'ADD_MESSAGE',
    sessionId,
    message: {
      id: uuid(),
      kind: 'assistant',
      text: message,
      reasoningText: '',
      streaming: false,
      timestamp: Date.now(),
    },
  });
}

export function useComposerActions({
  activeProject,
  projects,
  dispatch,
  phaseRef,
  activeSessionIdRef,
  pendingSubmitSessionRef,
  turnSessionMapRef,
  releasePendingSubmitSession,
  setConfirmDialog,
  refreshSessions,
  createSession,
  submitPrompt,
  interrupt,
  compactSession,
  deleteSession,
  deleteProject,
}: UseComposerActionsOptions) {
  const handleNewProject = useCallback(
    async (workingDir: string) => {
      try {
        const created = await createSession(workingDir);
        rememberProject(created.workingDir);
        await refreshSessions({ preferredSessionId: created.sessionId });
      } catch (error) {
        logger.error('useComposerActions', 'Failed to create project session:', error);
      }
    },
    [createSession, refreshSessions]
  );

  const handleNewSession = useCallback(async () => {
    if (!activeProject?.workingDir) {
      return;
    }
      try {
        const created = await createSession(activeProject.workingDir);
        await refreshSessions({ preferredSessionId: created.sessionId });
      } catch (error) {
        logger.error('useComposerActions', 'Failed to create session:', error);
      }
  }, [activeProject?.workingDir, createSession, refreshSessions]);

  const handleDeleteProject = useCallback(
    (projectId: string) => {
      const project = projects.find((item) => item.id === projectId);
      if (!project) {
        return;
      }

      setConfirmDialog({
        title: '删除项目',
        message: `删除项目"${project.name}"会移除该目录下所有会话，是否继续？`,
        danger: true,
        onConfirm: async () => {
          setConfirmDialog(null);
          try {
            const result = await deleteProject(project.workingDir);
            if (result.failedSessionIds.length > 0) {
              logger.error('useComposerActions', '部分会话删除失败:', result.failedSessionIds);
            }
            forgetProject(project.workingDir);
            await refreshSessions();
          } catch (error) {
            logger.error('useComposerActions', 'Failed to delete project:', error);
          }
        },
      });
    },
    [deleteProject, projects, refreshSessions, setConfirmDialog]
  );

  const handleDeleteSession = useCallback(
    (sessionId: string) => {
      setConfirmDialog({
        title: '删除会话',
        message: '确认删除该会话？该操作不可恢复。',
        danger: true,
        onConfirm: async () => {
          setConfirmDialog(null);
          try {
            await deleteSession(sessionId);
            await refreshSessions();
          } catch (error) {
            logger.error('useComposerActions', 'Failed to delete session:', error);
          }
        },
      });
    },
    [deleteSession, refreshSessions, setConfirmDialog]
  );

  const handleSubmit = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) {
        return;
      }

      const slashCommand = parseRuntimeSlashCommand(trimmed);
      const sessionId = activeSessionIdRef.current;
      if (!sessionId) {
        if (slashCommand) {
          setConfirmDialog({
            title: '无法执行命令',
            message: '当前没有激活会话，无法执行 `/compact`。',
            confirmLabel: '知道了',
            cancelLabel: '关闭',
            onConfirm: () => {
              setConfirmDialog(null);
            },
          });
        }
        return;
      }

      if (slashCommand) {
        if (slashCommand.kind === 'compactInvalidArgs') {
          appendLocalError(
            dispatch,
            sessionId,
            '`/compact` 当前不接受参数，请直接输入 `/compact`。'
          );
          return;
        }

        try {
          const acceptance = await compactSession(sessionId, { manualCompact: true });
          if (acceptance.deferred) {
            appendLocalNotice(dispatch, sessionId, acceptance.message);
          }
        } catch (error) {
          appendLocalError(
            dispatch,
            sessionId,
            error instanceof Error ? error.message : String(error)
          );
        }
        return;
      }

      if (phaseRef.current !== 'idle') {
        return;
      }

      // 在请求真正发出前就切到 busy，封住同一事件循环内的双击重入窗口。
      phaseRef.current = 'thinking';
      dispatch({ type: 'SET_PHASE', phase: 'thinking' });
      pendingSubmitSessionRef.current.push(sessionId);

      try {
        const submitted = await submitPrompt(sessionId, trimmed);
        const effectiveSessionId = submitted.sessionId ?? sessionId;
        turnSessionMapRef.current[submitted.turnId] =
          turnSessionMapRef.current[submitted.turnId] ?? effectiveSessionId;
        releasePendingSubmitSession(sessionId);

        if (
          submitted.branchedFromSessionId &&
          submitted.branchedFromSessionId === sessionId &&
          effectiveSessionId !== sessionId
        ) {
          // 分叉成功后旧 session 的 turnDone 可能已经在切换期间到达并被忽略；
          // 先本地兜底回 idle，避免 UI 把"正在思考"状态卡死到下一次刷新。
          phaseRef.current = 'idle';
          dispatch({ type: 'SET_PHASE', phase: 'idle' });
          await refreshSessions({ preferredSessionId: effectiveSessionId });
        }
      } catch (error) {
        releasePendingSubmitSession(sessionId);
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: `错误：${String(error)}`,
            reasoningText: '',
            streaming: false,
            timestamp: Date.now(),
          },
        });
        phaseRef.current = 'idle';
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
      }
    },
    [
      activeSessionIdRef,
      compactSession,
      dispatch,
      pendingSubmitSessionRef,
      phaseRef,
      refreshSessions,
      releasePendingSubmitSession,
      setConfirmDialog,
      submitPrompt,
      turnSessionMapRef,
    ]
  );

  const handleInterrupt = useCallback(async () => {
    const sessionId = activeSessionIdRef.current;
    if (!sessionId) {
      return;
    }
    await interrupt(sessionId);
  }, [activeSessionIdRef, interrupt]);

  return {
    handleDeleteProject,
    handleDeleteSession,
    handleInterrupt,
    handleNewProject,
    handleNewSession,
    handleSubmit,
  };
}
