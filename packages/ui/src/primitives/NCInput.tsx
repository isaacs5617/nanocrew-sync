import React from 'react';
import { getTokens, NC_FONT_UI, NC_FONT_MONO, type Theme } from '../tokens.js';

interface NCInputProps {
  value?: string;
  placeholder?: string;
  mono?: boolean;
  theme?: Theme;
  type?: string;
  style?: React.CSSProperties;
  onChange?: (value: string) => void;
  prefix?: React.ReactNode;
  suffix?: React.ReactNode;
  small?: boolean;
}

export const NCInput: React.FC<NCInputProps> = ({
  value, placeholder, mono, theme = 'dark', type = 'text',
  style, onChange, prefix, suffix, small,
}) => {
  const t = getTokens(theme);
  return (
    <div style={{
      display: 'flex', alignItems: 'center',
      background: t.surface1, border: `1px solid ${t.border}`,
      borderRadius: 3, padding: prefix ?? suffix ? '0 10px' : 0,
      ...style,
    }}>
      {prefix && (
        <div style={{ color: t.textMd, marginRight: 8, display: 'flex' }}>{prefix}</div>
      )}
      <input
        type={type}
        value={value ?? ''}
        placeholder={placeholder}
        onChange={(e) => onChange?.(e.target.value)}
        style={{
          flex: 1, minWidth: 0,
          background: 'transparent', border: 'none', outline: 'none',
          color: t.textHi, padding: small ? '8px 10px' : '11px 12px',
          fontFamily: mono ? NC_FONT_MONO : NC_FONT_UI,
          fontSize: mono ? 12 : 13, letterSpacing: mono ? 0 : -0.1,
        }}
      />
      {suffix && (
        <div style={{ color: t.textMd, marginLeft: 8, display: 'flex' }}>{suffix}</div>
      )}
    </div>
  );
};
