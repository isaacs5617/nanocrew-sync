import React from 'react';
import { getTokens, NC_FONT_UI, type Theme } from '../tokens.js';

interface NCBtnProps {
  children?: React.ReactNode;
  primary?: boolean;
  danger?: boolean;
  ghost?: boolean;
  small?: boolean;
  icon?: React.ReactNode;
  iconLeft?: React.ReactNode;
  onClick?: () => void;
  style?: React.CSSProperties;
  theme?: Theme;
  disabled?: boolean;
}

export const NCBtn: React.FC<NCBtnProps> = ({
  children, primary, danger, ghost, small, icon, iconLeft,
  onClick, style, theme = 'dark', disabled,
}) => {
  const t = getTokens(theme);
  let bg: string, color: string, border: string;
  if (primary) {
    bg = t.lime; color = theme === 'dark' ? '#0A0A0A' : '#FFFFFF'; border = t.lime;
  } else if (danger) {
    bg = 'transparent'; color = t.danger; border = t.border;
  } else if (ghost) {
    bg = 'transparent'; color = t.textHi; border = 'transparent';
  } else {
    bg = t.surface2; color = t.textHi; border = t.border;
  }
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        display: 'inline-flex', alignItems: 'center', gap: 8,
        padding: small ? '6px 12px' : '9px 16px',
        background: bg, color, border: `1px solid ${border}`,
        borderRadius: 3, cursor: disabled ? 'default' : 'pointer',
        fontFamily: NC_FONT_UI, fontSize: small ? 12 : 13, fontWeight: 600,
        letterSpacing: -0.1, opacity: disabled ? 0.4 : 1,
        transition: 'all 120ms',
        ...style,
      }}
    >
      {iconLeft}
      {children}
      {icon}
    </button>
  );
};
