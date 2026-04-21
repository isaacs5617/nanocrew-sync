import React from 'react';
import { NC_DARK, NC_LIGHT } from '../tokens.js';

interface NCMarkProps {
  size?: number;
  dark?: boolean;
}

export const NCMark: React.FC<NCMarkProps> = ({ size = 32, dark = true }) => {
  const stroke = dark ? NC_DARK.lime : NC_LIGHT.lime;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      style={{ display: 'block' }}
    >
      <polygon
        points="24,4 42,13.5 42,34.5 24,44 6,34.5 6,13.5"
        stroke={stroke}
        strokeWidth="2.5"
        fill="none"
      />
      <circle cx="24" cy="24" r="5" fill={stroke} />
    </svg>
  );
};
