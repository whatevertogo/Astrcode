export interface AssistantContentParts {
  text: string;
  reasoningText?: string;
}

export function splitAssistantContent(
  text: string,
  reasoningContent?: string | null
): AssistantContentParts {
  const inlineBlocks: string[] = [];
  const strippedText = text.replace(/<think>([\s\S]*?)<\/think>/gi, (_match, content: string) => {
    const normalized = content.trim();
    if (normalized) {
      inlineBlocks.push(normalized);
    }
    return '';
  });

  const inlineReasoning = inlineBlocks.length > 0 ? inlineBlocks.join('\n\n') : undefined;
  const explicitReasoning = reasoningContent?.trim() || undefined;
  const reasoningText =
    explicitReasoning && inlineReasoning && explicitReasoning !== inlineReasoning
      ? `${explicitReasoning}\n\n${inlineReasoning}`
      : (explicitReasoning ?? inlineReasoning);

  return {
    text: inlineBlocks.length > 0 ? strippedText.trim().replace(/\n{3,}/g, '\n\n') : text,
    reasoningText,
  };
}
