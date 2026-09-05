// Early P11 experiment: static documents, opaque origin, no scripts or navigations.
export function previewDocument(html: string) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("script,iframe,object,embed,base,meta,link").forEach((n) => n.remove());
  doc.querySelectorAll("*").forEach((node) => {
    for (const attr of [...node.attributes])
      if (/^(on|src|href|action|formaction|ping|target|srcdoc)/i.test(attr.name))
        node.removeAttribute(attr.name);
  });
  return (
    "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'none'; connect-src 'none'; form-action 'none'; base-uri 'none'\"><style>body{font:16px system-ui;padding:24px;color:#202029;background:white}</style>" +
    doc.body.innerHTML
  );
}
