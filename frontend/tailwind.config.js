/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        stone: {
          900: '#0f111a',
          950: '#0a0c14',
        },
        deep: {
          50: '#e2e8f0',
          100: '#c8d0dc',
          200: '#9eaec4',
          300: '#6f8aa8',
          400: '#4a6a8a',
          500: '#2d4a6a',
          600: '#1a3050',
          700: '#0f1e36',
          800: '#0a1424',
          900: '#060d18',
          950: '#030810',
        },
        bedrock: {
          50: '#ecfeff',
          100: '#cffafe',
          200: '#a5f3fc',
          300: '#67e8f9',
          400: '#22d3ee',
          500: '#06b6d4',
          600: '#0891b2',
          700: '#0e7490',
          800: '#155e75',
          900: '#164e63',
          950: '#083344',
        },
        teal: {
          400: '#2dd4bf',
          500: '#14b8a6',
          600: '#0d9488',
        },
      },
      fontFamily: {
        minecraft: ['"Press Start 2P"', 'monospace'],
        body: ['Montserrat', 'system-ui', 'sans-serif'],
        mono: ['"JetBrains Mono"', '"Fira Code"', 'monospace'],
        terminal: ['"VT323"', '"JetBrains Mono"', 'monospace'],
      },
      boxShadow: {
        'block': '4px 4px 0px 0px rgba(0,0,0,0.5)',
        'block-sm': '2px 2px 0px 0px rgba(0,0,0,0.5)',
        'block-lg': '6px 6px 0px 0px rgba(0,0,0,0.5)',
        'glow-teal': '0 0 20px rgba(45,212,191,0.15)',
        'glow-blue': '0 0 20px rgba(34,211,238,0.15)',
        'inner-glow': 'inset 0 1px 0 rgba(255,255,255,0.05)',
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
