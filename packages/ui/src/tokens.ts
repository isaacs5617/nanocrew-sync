export type Theme = 'dark' | 'light';

export interface ColorTokens {
  lime: string;
  limeSoft: string;
  limeSoftStrong: string;
  bg: string;
  surface1: string;
  surface2: string;
  surface3: string;
  border: string;
  borderStrong: string;
  textHi: string;
  textMd: string;
  textLo: string;
  textFaint: string;
  danger: string;
  warn: string;
  ok: string;
}

export const NC_DARK: ColorTokens = {
  lime: '#C8FF00',
  limeSoft: 'rgba(200,255,0,0.08)',
  limeSoftStrong: 'rgba(200,255,0,0.16)',
  bg: '#0A0A0A',
  surface1: '#15181D',
  surface2: '#1B2129',
  surface3: '#22272F',
  border: '#2A3038',
  borderStrong: '#3A4048',
  textHi: '#E8E8E8',
  textMd: '#9A9A9A',
  textLo: '#555555',
  textFaint: '#2A3038',
  danger: '#FF4D4D',
  warn: '#FFB800',
  ok: '#C8FF00',
};

export const NC_LIGHT: ColorTokens = {
  lime: '#3a5200',
  limeSoft: 'rgba(58,82,0,0.08)',
  limeSoftStrong: 'rgba(58,82,0,0.14)',
  bg: '#F5F5F5',
  surface1: '#FFFFFF',
  surface2: '#FAFAFA',
  surface3: '#F0F0F0',
  border: '#E0E0E0',
  borderStrong: '#C8C8C8',
  textHi: '#0A0A0A',
  textMd: '#555555',
  textLo: '#888888',
  textFaint: '#C0C0C0',
  danger: '#C91B1B',
  warn: '#B8860B',
  ok: '#3a5200',
};

export const NC_FONT_DISPLAY = "'Syne', system-ui, sans-serif";
export const NC_FONT_UI = "'Inter', system-ui, sans-serif";
export const NC_FONT_MONO = "'DM Mono', 'SF Mono', ui-monospace, monospace";

export const GOOGLE_FONTS_URL =
  'https://fonts.googleapis.com/css2?family=Syne:wght@800&family=Inter:wght@400;500;600&family=DM+Mono:wght@400;500&display=swap';

export function getTokens(theme: Theme): ColorTokens {
  return theme === 'dark' ? NC_DARK : NC_LIGHT;
}
