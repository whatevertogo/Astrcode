import type { Dispatch } from 'react';
import type { Action, Project, Session } from '../types';
import { uuid } from '../utils/uuid';

export function useProjects(dispatch: Dispatch<Action>) {
  const addProject = (name: string, workingDir: string) => {
    const projectId = uuid();
    const sessionId = uuid();
    const session: Session = {
      id: sessionId,
      projectId,
      title: '新会话',
      createdAt: Date.now(),
      messages: [],
    };
    const project: Project = {
      id: projectId,
      name,
      workingDir,
      sessions: [session],
      isExpanded: true,
    };
    dispatch({ type: 'ADD_PROJECT', project });
  };

  const addSession = (projectId: string) => {
    const session: Session = {
      id: uuid(),
      projectId,
      title: '新会话',
      createdAt: Date.now(),
      messages: [],
    };
    dispatch({ type: 'ADD_SESSION', projectId, session });
  };

  return { addProject, addSession };
}
