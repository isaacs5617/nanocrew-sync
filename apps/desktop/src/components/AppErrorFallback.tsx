import React from 'react';
import { relaunch } from '@tauri-apps/plugin-process';

export const AppErrorFallback: React.FC = () => {
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', alignItems: 'center',
      justifyContent: 'center', height: '100vh',
      background: '#0A0A0A', color: '#E8E8E8', fontFamily: 'sans-serif',
      gap: 16,
    }}>
      <div style={{ fontSize: 18, fontWeight: 600 }}>Something went wrong</div>
      <div style={{ fontSize: 13, color: '#888', textAlign: 'center', maxWidth: 340 }}>
        An unexpected error occurred. The crash has been reported automatically.
      </div>
      <button
        onClick={() => relaunch().catch(() => {})}
        style={{
          marginTop: 8, padding: '10px 24px',
          background: '#C8FF00', color: '#0A0A0A',
          border: 'none', borderRadius: 3,
          fontSize: 13, fontWeight: 600, cursor: 'pointer',
        }}
      >
        Restart app
      </button>
    </div>
  );
};
