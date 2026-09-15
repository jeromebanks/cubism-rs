// Progressive enhancement only. Every document and diagram source is readable without JS.
const figures = [...document.querySelectorAll('.diagram')];
if (figures.length) {
  try {
    const { default: mermaid } = await import('https://cdn.jsdelivr.net/npm/mermaid@11.12.0/dist/mermaid.esm.min.mjs');
    mermaid.initialize({ startOnLoad:false, securityLevel:'strict', theme:'neutral',
      fontFamily:'system-ui, sans-serif', flowchart:{useMaxWidth:false}, sequence:{useMaxWidth:false} });
    for (const [index, figure] of figures.entries()) {
      try {
        const source = figure.querySelector('code').textContent;
        const { svg } = await mermaid.render(`nightshift-diagram-${index}`, source);
        figure.querySelector('.diagram-view').innerHTML = svg;
        figure.querySelector('.diagram-status').textContent = 'Scroll horizontally to explore the full diagram.';
        figure.querySelector('details').open = false;
      } catch {
        figure.querySelector('.diagram-status').textContent = 'Diagram rendering unavailable. The complete source is shown below.';
      }
    }
  } catch {
    for (const figure of figures) {
      figure.querySelector('.diagram-status').textContent = 'Offline or diagram library unavailable. The complete source is shown below.';
    }
  }
}
