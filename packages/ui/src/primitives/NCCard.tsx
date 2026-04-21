import React from 'react';
import { getTokens, type Theme } from '../tokens.js';

interface NCCardProps {
  children?: React.ReactNode;
  theme?: Theme;
  pad?: number;
  style?: React.CSSProperties;
  onClick?: () => void;
}

export const NCCard: React.FC<NCCardProps> = ({
  children, theme = 'dark', pad = 20, style, onClick,
}) => {
  const t = getTokens(theme);
  return (
    <div
      onClick={onClick}
      style={{
        background: t.surface1, border: `1px solid ${t.border}`,
        borderRadius: 4, padding: pad,
        cursor: onClick ? 'pointer' : 'default',
        ...style,
      }}
    >
      {children}
    </div>
  );
};
