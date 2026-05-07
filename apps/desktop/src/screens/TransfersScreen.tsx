import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  getTokens, NC_FONT_MONO, NC_FONT_UI,
  NCBtn, TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useTransfers } from '../context/transfers.js';

function formatBytes(b: number) {
  if (b < 1024) return `${b} B`;
  if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1073741824) return `${(b / 1048576).toFixed(1)} MB`;
  return `${(b / 1073741824).toFixed(2)} GB`;
}

function formatSpeed(bytes: number, ms: number) {
  if (ms <= 0) return '';
  const bps = (bytes / ms) * 1000;
  return `${formatBytes(bps)}/s`;
}

interface TransfersScreenProps { theme: Theme }

export const TransfersScreen: React.FC<TransfersScreenProps> = ({ theme }) => {
  const tok = getTokens(theme);
  const { t } = useTranslation();
  const { transfers, clearFinished } = useTransfers();
  // Tick every 500ms so the live speed readout updates while a transfer is in flight.
  const [, setTick] = React.useState(0);
  React.useEffect(() => {
    const id = setInterval(() => setTick(n => n + 1), 500);
    return () => clearInterval(id);
  }, []);

  const active   = transfers.filter(tr => tr.state === 'start' || tr.state === 'progress');
  const finished = transfers.filter(tr => tr.state === 'done' || tr.state === 'error');

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[t('transfers.crumb')]}
        title={<>{t('transfers.titlePrefix')} <span style={{ color: tok.lime }}>{t('transfers.titleAccent')}</span></>}
        subtitle={active.length > 0
          ? t(active.length !== 1 ? 'transfers.activeOther' : 'transfers.activeOne', { count: active.length })
          : t('transfers.subtitle')}
        actions={finished.length > 0
          ? <NCBtn theme={theme} small ghost onClick={clearFinished}>{t('transfers.clearDone')}</NCBtn>
          : undefined
        }
      />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {transfers.length === 0 ? (
          <div style={{
            flex: 1, display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center', gap: 16, padding: 40,
          }}>
            <I.upload size={36} color={tok.textLo} />
            <div style={{ fontSize: 15, fontWeight: 500, color: tok.textHi }}>{t('transfers.empty.title')}</div>
            <div style={{ fontSize: 13, color: tok.textMd, textAlign: 'center', maxWidth: 420, lineHeight: 1.6 }}>
              {t('transfers.empty.body')}
            </div>
          </div>
        ) : (
          <div style={{ flex: 1, overflow: 'auto' }}>
            {/* Header */}
            <div style={{
              display: 'grid', gridTemplateColumns: '20px 1fr 90px 200px 80px',
              gap: 12, padding: '9px 20px',
              borderBottom: `1px solid ${tok.border}`,
              fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
              color: tok.textMd, textTransform: 'uppercase',
              position: 'sticky', top: 0, background: tok.bg, zIndex: 1,
            }}>
              <span />
              <span>{t('transfers.col.file')}</span>
              <span>{t('transfers.col.size')}</span>
              <span>{t('transfers.col.progress')}</span>
              <span>{t('transfers.col.speed')}</span>
            </div>

            {transfers.map(xfer => {
              const pct = xfer.total_bytes > 0
                ? Math.min(100, (xfer.done_bytes / xfer.total_bytes) * 100)
                : (xfer.state === 'done' ? 100 : 0);
              const isDone  = xfer.state === 'done';
              const isError = xfer.state === 'error';
              const isActive = xfer.state === 'start' || xfer.state === 'progress';
              const elapsedMs = xfer.ended_at
                ? xfer.ended_at.getTime() - xfer.started_at.getTime()
                : Date.now() - xfer.started_at.getTime();
              const speed = isActive && xfer.done_bytes > 0
                ? formatSpeed(xfer.done_bytes, elapsedMs)
                : isDone
                  ? formatSpeed(xfer.total_bytes, elapsedMs)
                  : '';

              return (
                <div key={xfer.id} style={{
                  display: 'grid', gridTemplateColumns: '20px 1fr 90px 200px 80px',
                  gap: 12, padding: '10px 20px', alignItems: 'center',
                  borderBottom: `1px solid ${tok.border}`,
                  opacity: isError ? 0.7 : 1,
                }}>
                  {/* Direction icon */}
                  <div style={{ color: isError ? tok.danger : isActive ? tok.lime : tok.textLo }}>
                    {xfer.direction === 'upload'
                      ? <I.upload size={13} color={isError ? tok.danger : isActive ? tok.lime : tok.textLo} />
                      : <I.download size={13} color={isError ? tok.danger : isActive ? tok.lime : tok.textLo} />
                    }
                  </div>

                  {/* Filename */}
                  <div style={{
                    fontFamily: NC_FONT_UI, fontSize: 13,
                    color: isError ? tok.danger : tok.textHi,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                  }}>
                    {xfer.filename}
                    {isError && xfer.error && (
                      <span style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: tok.danger, marginLeft: 8 }}>
                        {xfer.error}
                      </span>
                    )}
                  </div>

                  {/* Size */}
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: tok.textMd }}>
                    {formatBytes(xfer.total_bytes)}
                  </div>

                  {/* Progress bar + label */}
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    <div style={{
                      height: 4, background: tok.surface2, borderRadius: 2, overflow: 'hidden',
                    }}>
                      <div style={{
                        height: '100%', borderRadius: 2,
                        width: `${pct}%`,
                        background: isError ? tok.danger : isDone ? tok.textLo : tok.lime,
                        transition: 'width 0.2s',
                      }} />
                    </div>
                    <div style={{ fontFamily: NC_FONT_MONO, fontSize: 9, color: tok.textMd }}>
                      {isError ? t('transfers.state.failed') : isDone ? t('transfers.state.done') : `${Math.round(pct)}% · ${formatBytes(xfer.done_bytes)}`}
                    </div>
                  </div>

                  {/* Speed */}
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: tok.textLo }}>
                    {speed}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
};
