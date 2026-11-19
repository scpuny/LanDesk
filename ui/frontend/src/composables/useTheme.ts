// 主题组合式 —— 移植自 lanLink useTheme.ts，改为纯前端（无 Tauri API）+ 类型声明。
// 两个独立维度：明暗 pref → data-theme；视觉风格 palette → data-palette。
import { ref, type Ref } from 'vue'
import { DEFAULT_PALETTE, isPaletteId } from '../data/themes'
import type { ThemePref, PaletteId } from '../types/index'

const STORAGE_KEY = 'ld-theme'
const PALETTE_KEY = 'ld-palette'

function systemTheme(): 'light' | 'dark' {
  if (typeof window === 'undefined' || !window.matchMedia) return 'light'
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

function applyToDom(pref: ThemePref): void {
  const resolved = pref === 'system' ? systemTheme() : pref
  document.documentElement.setAttribute('data-theme', resolved)
}

function applyPalette(id: PaletteId): void {
  const root = document.documentElement
  if (id === DEFAULT_PALETTE) root.removeAttribute('data-palette')
  else root.setAttribute('data-palette', id)
}

function initialPalette(): PaletteId {
  const fromUrl = new URLSearchParams(window.location.search).get('palette')
  if (isPaletteId(fromUrl)) return fromUrl
  const saved = localStorage.getItem(PALETTE_KEY)
  if (isPaletteId(saved)) return saved
  if (saved) localStorage.removeItem(PALETTE_KEY)
  return DEFAULT_PALETTE
}

function initialTheme(): ThemePref {
  const fromUrl = new URLSearchParams(window.location.search).get('theme')
  if (fromUrl === 'light' || fromUrl === 'dark' || fromUrl === 'system') return fromUrl
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved === 'light' || saved === 'dark' || saved === 'system') return saved
  return 'system'
}

const pref: Ref<ThemePref> = ref<ThemePref>(initialTheme())
const palette: Ref<PaletteId> = ref<PaletteId>(initialPalette())

applyToDom(pref.value)
applyPalette(palette.value)

if (typeof window !== 'undefined' && window.matchMedia) {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (pref.value === 'system') applyToDom('system')
  })
}

export interface UseThemeReturn {
  pref: Ref<ThemePref>
  palette: Ref<PaletteId>
  setTheme: (p: ThemePref) => void
  setPalette: (id: PaletteId) => void
  resetPalette: () => void
  systemTheme: () => 'light' | 'dark'
  applyToDom: (pref: ThemePref) => void
}

export function useTheme(): UseThemeReturn {
  function setTheme(p: ThemePref): void {
    pref.value = p
    localStorage.setItem(STORAGE_KEY, p)
    applyToDom(p)
  }
  function setPalette(id: PaletteId): void {
    palette.value = id
    localStorage.setItem(PALETTE_KEY, id)
    applyPalette(id)
  }
  function resetPalette(): void { setPalette(DEFAULT_PALETTE) }
  return { pref, setTheme, systemTheme, applyToDom, palette, setPalette, resetPalette }
}
