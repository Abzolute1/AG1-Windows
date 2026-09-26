import { useRef, useState } from "react";
import { ChevronDown, Download, Upload, Check } from "lucide-react";
import { builtinThemes, resolveTheme, type ThemePlugin } from "./themes";
import { type Preferences } from "./preferences";
import { api } from "./bridge";
export function ThemeSettings({ theme, setTheme, plugins, refresh, preferences: p, update }: { theme: string; setTheme: (id: string) => void; plugins: ThemePlugin[]; refresh: () => Promise<void>; preferences: Preferences; update: (patch: Partial<Preferences>) => void }) {
  const [open, setOpen] = useState(false), [error, setError] = useState(""), [notice, setNotice] = useState(""), [working, setWorking] = useState(false);
  const input = useRef<HTMLInputElement>(null), selected = resolveTheme(theme, plugins);
  async function importFile(file?: File) {
    if (!file) return;
    setError(""); setNotice(""); setWorking(true);
    try { if (file.size > 32768) throw new Error("Theme files must be smaller than 32 KB."); const plugin = JSON.parse(await file.text()); await api.importTheme(plugin); await refresh(); setTheme(plugin.id); update({accent:"",background:"",foreground:""}); setNotice(`${plugin.name} installed.`); }
    catch (e) { setError(String(e).replace(/^Error: /,"")); }
    finally { setWorking(false); if (input.current) input.current.value = ""; }
  }
  function exportColors() {
    const colors: Record<string,string> = { ...selected.colors, ...(p.accent ? {primary:p.accent,ring:p.accent,link:p.accent} : {}), ...(p.background ? {background:p.background} : {}), ...(p.foreground ? {foreground:p.foreground} : {}) };
    if (p.accent) { const [r,g,b]=p.accent.slice(1).match(/../g)!.map(c=>parseInt(c,16)); colors['primary-foreground']=r*.299+g*.587+b*.114>150?'#151518':'#ffffff'; }
    return colors;
  }
  async function copy() {
    const colors = exportColors();
    const plugin = { ...selected, id: `custom-${selected.id}`.slice(0,80), name: `${selected.name} custom`.slice(0,80), colors };
    try { await navigator.clipboard.writeText(JSON.stringify(plugin, null, 2)); setNotice("Theme copied. Save it as an .ag1-theme.json file to share or import."); setError(""); } catch { setError("Clipboard is unavailable. Use Export to save the theme file."); }
    return plugin;
  }
  async function exportFile() {
    const colors = exportColors();
    const plugin = { ...selected, id: `custom-${selected.id}`.slice(0,80), name: `${selected.name} custom`.slice(0,80), colors };
    setWorking(true);setError("");try { const path = await api.exportTheme(plugin);setNotice(`Theme saved to ${path}`); } catch(e) {setError(String(e));} finally {setWorking(false);}
  }
  return <><div className="theme-code-preview" aria-label="Theme preview"><div><span>01</span><code><b>const</b> appearance = {'{'}</code></div><div><span>02</span><code>  theme: <em>"{selected.name}"</em>,</code></div><div><span>03</span><code>  background: <em>"{p.background || selected.colors.background}"</em></code></div><div><span>04</span><code>{'}'};</code></div></div>
    <div className="settings-card theme-editor"><div className="theme-editor-header"><strong>Theme</strong><div className="theme-actions"><button className="quiet-button" disabled={working} onClick={()=>input.current?.click()}><Upload size={14}/>Import</button><button className="quiet-button" onClick={()=>void copy()}>Copy theme</button><button className="quiet-button" aria-label="Export theme" title="Export theme" disabled={working} onClick={()=>void exportFile()}><Download size={15}/></button></div><div className="theme-picker-anchor" onKeyDown={e=>{if(e.key==="Escape"){setOpen(false);e.stopPropagation();}}}><button className="theme-picker" aria-label="Choose theme" aria-expanded={open} onClick={()=>setOpen(!open)}><span className="theme-monogram">Aa</span>{theme === 'system' ? 'System' : selected.name}<ChevronDown size={14}/></button>{open && <><button className="theme-dismiss" aria-label="Close theme menu" onClick={()=>setOpen(false)}/><div className="theme-library menu" role="listbox" aria-label="Themes">{[{id:'system',name:'System',colors:selected.colors},...builtinThemes,...plugins].map(t=><button key={t.id} role="option" aria-selected={theme===t.id} onClick={()=>{setTheme(t.id);update({accent:"",background:"",foreground:""});setOpen(false);}}><span className="theme-monogram" style={{background:t.colors.background,color:t.colors.link}}>Aa</span><span>{t.name}</span>{theme===t.id&&<Check size={14}/>}</button>)}</div></>}</div></div>
    {([['accent','Accent','primary'],['background','Background','background'],['foreground','Foreground','foreground']] as const).map(([key,label,token])=><div className="setting-row" key={key}><span>{label}</span><div className="setting-control">{p[key]&&<button className="quiet-button" onClick={()=>update({[key]:''})}>Reset</button>}<label className="color-control"><input type="color" aria-label={`${label} color`} value={p[key]||selected.colors[token]} onChange={e=>update({[key]:e.target.value})}/><span>{p[key]||selected.colors[token]}</span></label></div></div>)}
    </div><input ref={input} type="file" accept=".json,.ag1-theme.json,application/json" aria-label="Import theme file" hidden onChange={e=>void importFile(e.target.files?.[0])}/>
    {error&&<p role="alert" className="settings-error">{error}</p>}{notice&&<p role="status" className="settings-footnote">{notice}</p>}
    {!!plugins.length&&<details className="theme-installed"><summary>Installed theme plugins · {plugins.length}</summary>{plugins.map(t=><div className="setting-row" key={t.id}><span>{t.name}</span><button className="quiet-button" disabled={working} onClick={async()=>{setWorking(true);try{await api.removeTheme(t.id);if(theme===t.id)setTheme('dark');await refresh();}catch(e){setError(String(e));}finally{setWorking(false);}}}>Remove</button></div>)}</details>}
    <p className="settings-footnote">Import a downloaded .ag1-theme.json plugin, or export your palette to share it. Theme plugins contain colors only.</p></>;
}
