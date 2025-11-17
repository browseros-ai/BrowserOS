/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    './app/**/*.{js,jsx,ts,tsx}',
    './components/**/*.{js,jsx,ts,tsx}',
  ],
  theme: {
    extend: {
      colors: {
        'primary': '#667eea',
        'primary-hover': '#5568d3',
        'secondary': '#764ba2',
      },
    },
  },
  plugins: [],
};
