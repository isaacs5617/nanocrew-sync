import React from 'react';
import { NC_DARK, NC_LIGHT, NC_FONT_DISPLAY, NC_FONT_MONO } from '../tokens.js';

interface NCWordmarkProps {
  dark?: boolean;
  size?: number;
}

export const NCWordmark: React.FC<NCWordmarkProps> = ({ dark = true, size = 16 }) => {
  const t = dark ? NC_DARK : NC_LIGHT;
  return (
    <div style={{
      fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
      fontSize: size, letterSpacing: -0.5,
      color: t.textHi, display: 'inline-flex', alignItems: 'baseline', gap: 8,
    }}>
      <span>
        Nano<span style={{ color: t.lime }}>Crew</span>
      </span>
      <span style={{
        fontFamily: NC_FONT_MONO, fontWeight: 500,
        fontSize: size * 0.55, letterSpacing: 2.5, color: t.textMd,
      }}>
        SYNC
      </span>
    </div>
  );
};
