import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
export default defineConfig({plugins:[react(),tailwindcss(),{name:'ag1-bundle-provenance',generateBundle(_options,bundle){this.emitFile({type:'asset',fileName:'bundle-provenance.json',source:JSON.stringify(Object.fromEntries(Object.entries(bundle).map(([name,item])=>[name,item.type==='chunk'?{type:item.type,modules:Object.keys(item.modules),imports:item.imports,dynamicImports:item.dynamicImports}:{type:item.type,name:item.name} ])),null,2)});}}],resolve:{alias:{'@':fileURLToPath(new URL('./src',import.meta.url))}},server:{port:1420,strictPort:true},build:{target:'es2022'},test:{environment:'jsdom',include:['src/**/*.test.ts','src/**/*.test.tsx']}});
