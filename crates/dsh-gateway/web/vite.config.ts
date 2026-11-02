import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  base: '/_dsh_login/',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
