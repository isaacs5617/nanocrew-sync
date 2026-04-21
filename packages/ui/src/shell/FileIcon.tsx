import React from 'react';
import { getTokens, type Theme } from '../tokens.js';
import { I } from '../icons.js';
import type { FileEntry } from '../types.js';

interface FileIconProps {
  kind: FileEntry['kind'];
  size?: number;
  theme?: Theme;
}

export const FileIcon: React.FC<FileIconProps> = ({ kind, size = 16, theme = 'dark' }) => {
  const t = getTokens(theme);
  const map: Record<FileEntry['kind'], React.ReactElement> = {
    folder: <I.folder size={size} color={t.lime} />,
    image:  <I.fileImg size={size} color={t.textMd} />,
    video:  <I.fileVid size={size} color={t.textMd} />,
    doc:    <I.fileDoc size={size} color={t.textMd} />,
    file:   <I.file size={size} color={t.textMd} />,
  };
  return map[kind] ?? map.file;
};
