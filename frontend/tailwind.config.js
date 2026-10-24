/**
 * OmniBedrock design system — Tailwind bridge.
 *
 * Every palette below resolves to CSS custom properties defined in src/app.css.
 * That keeps components theme-agnostic: switching accent theme or light/dark
 * mode only swaps variable values on <html data-theme data-mode>, never a
 * rebuild. Color channels are space-separated RGB triplets so Tailwind's
 * opacity modifiers (`bg-deep-800/60`) keep working via <alpha-value>.
 *
 * Palette roles:
 *   deep     — neutral surfaces & text (flips per mode; "minecraft" theme
 *              swaps in an earthy ramp)
 *   bedrock  — the ACCENT ramp; re-pointed per theme (cyan, green, blue, …)
 *   strong   — highest-emphasis text (near-white in dark, near-black in light)
 *   overlay  — modal scrims
 *   red/green/yellow/teal/blue/purple/indigo — semantic status colors,
 *              hand-tuned per mode for readable contrast
 */
const rgb = (name) => `rgb(var(${name}) / <alpha-value>)`;

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  darkMode: ['class', '[data-mode="dark"]'],
  theme: {
    extend: {
      colors: {
        strong: { DEFAULT: rgb('--color-strong') },
        overlay: { DEFAULT: rgb('--color-overlay') },
        deep: {
          50: rgb('--deep-50'),
          100: rgb('--deep-100'),
          200: rgb('--deep-200'),
          300: rgb('--deep-300'),
          400: rgb('--deep-400'),
          500: rgb('--deep-500'),
          600: rgb('--deep-600'),
          700: rgb('--deep-700'),
          800: rgb('--deep-800'),
          900: rgb('--deep-900'),
          950: rgb('--deep-950'),
        },
        bedrock: {
          50: rgb('--bedrock-50'),
          100: rgb('--bedrock-100'),
          200: rgb('--bedrock-200'),
          300: rgb('--bedrock-300'),
          400: rgb('--bedrock-400'),
          500: rgb('--bedrock-500'),
          600: rgb('--bedrock-600'),
          700: rgb('--bedrock-700'),
          800: rgb('--bedrock-800'),
          900: rgb('--bedrock-900'),
          950: rgb('--bedrock-950'),
        },
        red: {
          100: rgb('--c-red-100'),
          200: rgb('--c-red-200'),
          300: rgb('--c-red-300'),
          400: rgb('--c-red-400'),
          500: rgb('--c-red-500'),
          600: rgb('--c-red-600'),
          700: rgb('--c-red-700'),
          900: rgb('--c-red-900'),
          950: rgb('--c-red-950'),
        },
        green: {
          300: rgb('--c-green-300'),
          400: rgb('--c-green-400'),
          500: rgb('--c-green-500'),
          700: rgb('--c-green-700'),
          900: rgb('--c-green-900'),
        },
        yellow: {
          300: rgb('--c-yellow-300'),
          400: rgb('--c-yellow-400'),
          500: rgb('--c-yellow-500'),
          600: rgb('--c-yellow-600'),
          700: rgb('--c-yellow-700'),
          900: rgb('--c-yellow-900'),
        },
        teal: {
          300: rgb('--c-teal-300'),
          400: rgb('--c-teal-400'),
          500: rgb('--c-teal-500'),
          600: rgb('--c-teal-600'),
          700: rgb('--c-teal-700'),
          950: rgb('--c-teal-950'),
        },
        blue: { 400: rgb('--c-blue-400') },
        purple: { 400: rgb('--c-purple-400') },
        indigo: { 400: rgb('--c-indigo-400') },
      },
      borderRadius: {
        DEFAULT: '2px',
        sm: '2px',
      },
      fontFamily: {
        minecraft: ['"Press Start 2P"', 'monospace'],
        body: ['Montserrat', 'system-ui', 'sans-serif'],
        mono: ['"JetBrains Mono"', '"Fira Code"', 'monospace'],
        terminal: ['"JetBrains Mono"', '"VT323"', 'monospace'],
      },
      boxShadow: {
        // Classic hard "block" shadows (Minecraft-style). Color comes from a
        // token so light mode can soften them.
        block: '3px 3px 0 0 var(--shadow-block)',
        'block-sm': '2px 2px 0 0 var(--shadow-block)',
        'block-lg': '5px 5px 0 0 var(--shadow-block)',
        'glow-accent': '0 0 16px rgb(var(--bedrock-500) / 0.25)',
        'inner-glow': 'inset 0 1px 0 rgb(255 255 255 / 0.05)',
      },
      animation: {
        'pulse-slow': 'pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite',
        'fade-in': 'fadeIn 0.2s ease-out',
        'slide-up': 'slideUp 0.3s ease-out',
      },
      keyframes: {
        fadeIn: { '0%': { opacity: '0' }, '100%': { opacity: '1' } },
        slideUp: { '0%': { opacity: '0', transform: 'translateY(8px)' }, '100%': { opacity: '1', transform: 'translateY(0)' } },
      },
    },
  },
  plugins: [],
};
