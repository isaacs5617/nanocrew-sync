import React from 'react';

interface IconProps {
  size?: number;
  color?: string;
  strokeWidth?: number;
  style?: React.CSSProperties;
}

const NCIcon: React.FC<IconProps & { children: React.ReactNode }> = ({
  children,
  size = 16,
  color = 'currentColor',
  strokeWidth = 1.5,
  style,
}) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke={color}
    strokeWidth={strokeWidth}
    strokeLinecap="round"
    strokeLinejoin="round"
    style={{ display: 'block', flexShrink: 0, ...style }}
  >
    {children}
  </svg>
);

export type Icon = React.FC<IconProps>;

export const I: Record<string, Icon> = {
  cloud:    (p) => <NCIcon {...p}><path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9z"/></NCIcon>,
  folder:   (p) => <NCIcon {...p}><path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6z"/></NCIcon>,
  file:     (p) => <NCIcon {...p}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></NCIcon>,
  fileImg:  (p) => <NCIcon {...p}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><circle cx="9" cy="13" r="1.5"/><path d="M20 17l-4-4-7 7"/></NCIcon>,
  fileVid:  (p) => <NCIcon {...p}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><polygon points="10 13 10 19 16 16 10 13"/></NCIcon>,
  fileDoc:  (p) => <NCIcon {...p}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/></NCIcon>,
  plus:     (p) => <NCIcon {...p}><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></NCIcon>,
  check:    (p) => <NCIcon {...p} strokeWidth={2}><polyline points="20 6 9 17 4 12"/></NCIcon>,
  x:        (p) => <NCIcon {...p} strokeWidth={2}><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></NCIcon>,
  arrow:    (p) => <NCIcon {...p} strokeWidth={2}><line x1="5" y1="12" x2="19" y2="12"/><polyline points="12 5 19 12 12 19"/></NCIcon>,
  chevL:    (p) => <NCIcon {...p} strokeWidth={2}><polyline points="15 18 9 12 15 6"/></NCIcon>,
  chevR:    (p) => <NCIcon {...p} strokeWidth={2}><polyline points="9 18 15 12 9 6"/></NCIcon>,
  chevD:    (p) => <NCIcon {...p} strokeWidth={2}><polyline points="6 9 12 15 18 9"/></NCIcon>,
  settings: (p) => <NCIcon {...p}><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></NCIcon>,
  search:   (p) => <NCIcon {...p}><circle cx="11" cy="11" r="7"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></NCIcon>,
  refresh:  (p) => <NCIcon {...p}><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-.18-8.58"/></NCIcon>,
  upload:   (p) => <NCIcon {...p}><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></NCIcon>,
  download: (p) => <NCIcon {...p}><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="3" x2="12" y2="15"/></NCIcon>,
  lock:     (p) => <NCIcon {...p}><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></NCIcon>,
  globe:    (p) => <NCIcon {...p}><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15 15 0 0 1 0 20 15 15 0 0 1 0-20z"/></NCIcon>,
  dot:      (p) => <NCIcon {...p} strokeWidth={0}><circle cx="12" cy="12" r="4" fill={p?.color ?? 'currentColor'}/></NCIcon>,
  link:     (p) => <NCIcon {...p}><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></NCIcon>,
  drive:    (p) => <NCIcon {...p}><rect x="2" y="4" width="20" height="16" rx="2"/><circle cx="7" cy="12" r="1.5" fill="currentColor" stroke="none"/><line x1="11" y1="12" x2="19" y2="12"/></NCIcon>,
  serverDb: (p) => <NCIcon {...p}><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v6a9 3 0 0 0 18 0V5"/><path d="M3 11v6a9 3 0 0 0 18 0v-6"/></NCIcon>,
  user:     (p) => <NCIcon {...p}><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></NCIcon>,
  menu:     (p) => <NCIcon {...p}><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></NCIcon>,
  more:     (p) => <NCIcon {...p}><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="19" cy="12" r="1.2" fill="currentColor"/><circle cx="5" cy="12" r="1.2" fill="currentColor"/></NCIcon>,
  trash:    (p) => <NCIcon {...p}><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6M10 11v6M14 11v6M9 6V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/></NCIcon>,
  warn:     (p) => <NCIcon {...p}><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></NCIcon>,
  pause:    (p) => <NCIcon {...p} strokeWidth={2}><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></NCIcon>,
  play:     (p) => <NCIcon {...p} strokeWidth={2}><polygon points="5 3 19 12 5 21 5 3"/></NCIcon>,
  eye:      (p) => <NCIcon {...p}><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></NCIcon>,
  eyeOff:   (p) => <NCIcon {...p}><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></NCIcon>,
  pin:      (p) => <NCIcon {...p}><path d="M21 10c0 7-9 13-9 13s-9-6-9-13a9 9 0 0 1 18 0z"/><circle cx="12" cy="10" r="3"/></NCIcon>,
  bell:     (p) => <NCIcon {...p}><path d="M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></NCIcon>,
  home:     (p) => <NCIcon {...p}><path d="M3 12L12 3l9 9M5 10v10a1 1 0 0 0 1 1h3v-6h6v6h3a1 1 0 0 0 1-1V10"/></NCIcon>,
  activity: (p) => <NCIcon {...p}><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></NCIcon>,
  clock:    (p) => <NCIcon {...p}><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></NCIcon>,
  shield:   (p) => <NCIcon {...p}><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></NCIcon>,
  wifi:     (p) => <NCIcon {...p}><path d="M5 12.55a11 11 0 0 1 14.08 0M1.42 9a16 16 0 0 1 21.16 0M8.53 16.11a6 6 0 0 1 6.95 0M12 20h.01"/></NCIcon>,
  offline:  (p) => <NCIcon {...p}><line x1="1" y1="1" x2="23" y2="23"/><path d="M16.72 11.06A10.94 10.94 0 0 1 19 12.55M5 12.55a10.94 10.94 0 0 1 5.17-2.39M10.71 5.05A16 16 0 0 1 22.58 9M1.42 9a15.91 15.91 0 0 1 4.7-2.88M8.53 16.11a6 6 0 0 1 6.95 0M12 20h.01"/></NCIcon>,
  s3:       (p) => <NCIcon {...p}><path d="M3 5a9 3 0 0 1 18 0v14a9 3 0 0 1-18 0z"/><path d="M3 5c0 1.66 4 3 9 3s9-1.34 9-3"/></NCIcon>,
};
