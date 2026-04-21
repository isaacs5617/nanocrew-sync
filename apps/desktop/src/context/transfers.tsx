import React from 'react';
import { listen } from '@tauri-apps/api/event';

export interface TransferPayload {
  id: number;
  drive_id: number;
  filename: string;
  direction: 'upload' | 'download';
  total_bytes: number;
  done_bytes: number;
  state: 'start' | 'progress' | 'done' | 'error';
  error?: string;
}

export interface Transfer extends TransferPayload {
  started_at: Date;
  ended_at?: Date;
}

interface Ctx {
  transfers: Transfer[];
  clearFinished: () => void;
}

const TransfersCtx = React.createContext<Ctx>({ transfers: [], clearFinished: () => {} });

export const TransfersProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [transfers, setTransfers] = React.useState<Transfer[]>([]);

  React.useEffect(() => {
    const unlisten = listen<TransferPayload>('transfer_progress', e => {
      const p = e.payload;
      setTransfers(prev => {
        const existing = prev.find(x => x.id === p.id);
        if (!existing) {
          if (p.state === 'start' || p.state === 'progress') {
            return [{ ...p, started_at: new Date() }, ...prev].slice(0, 200);
          }
          return prev;
        }
        return prev.map(x => x.id === p.id
          ? {
              ...x,
              done_bytes: p.done_bytes,
              state: p.state,
              error: p.error,
              ended_at: (p.state === 'done' || p.state === 'error') ? new Date() : x.ended_at,
            }
          : x
        );
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const clearFinished = React.useCallback(() => {
    setTransfers(prev => prev.filter(x => x.state === 'start' || x.state === 'progress'));
  }, []);

  return (
    <TransfersCtx.Provider value={{ transfers, clearFinished }}>
      {children}
    </TransfersCtx.Provider>
  );
};

export const useTransfers = () => React.useContext(TransfersCtx);
