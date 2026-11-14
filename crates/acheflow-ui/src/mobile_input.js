// Canvas text fields need a real browser editor to open the mobile keyboard.
export function installMobileInput(open, update) {
  let start = null;
  let panel = null;
  let editor = null;
  let active = null;
  const close = () => {
    if (editor) editor.value = '';
    if (panel) panel.remove();
    panel = editor = active = null;
  };
  const position = () => {
    if (!panel) return;
    const view = window.visualViewport;
    panel.style.bottom = `${view ? Math.max(0, window.innerHeight - view.height - view.offsetTop) : 0}px`;
  };
  const down = e => {
    if (e.target.tagName === 'CANVAS' && e.isPrimary && (e.pointerType === 'touch' || e.pointerType === 'pen')) {
      start = {id: e.pointerId, x: e.clientX, y: e.clientY};
    } else start = null;
  };
  const up = e => {
    const touch = start;
    start = null;
    if (!touch || touch.id !== e.pointerId || Math.hypot(e.clientX-touch.x,e.clientY-touch.y) > 12 || e.target.tagName !== 'CANVAS') return;
    const rect = e.target.getBoundingClientRect();
    const data = open(e.clientX-rect.left, e.clientY-rect.top, rect.width, rect.height);
    if (!data) return;
    const field = JSON.parse(data);
    close();
    active = field.token;
    panel = document.createElement('section');
    panel.className = 'acheflow-mobile-editor';
    panel.setAttribute('role', 'dialog');
    panel.setAttribute('aria-label', field.label);
    const label = document.createElement('label');
    const title = document.createElement('span');
    title.textContent = field.label;
    editor = document.createElement(field.multiline ? 'textarea' : 'input');
    if (!field.multiline) editor.type = field.secret ? 'password' : 'text';
    editor.value = field.value;
    editor.maxLength = field.max_length;
    editor.autocomplete = 'off';
    editor.autocapitalize = 'off';
    editor.spellcheck = false;
    editor.inputMode = field.input_mode;
    editor.style.fontSize = '16px'; // Avoid iOS focus zoom.
    editor.addEventListener('keydown', e => {
      e.stopPropagation();
      if (e.key === 'Escape') { update(active, editor.value, true); close(); }
    });
    editor.addEventListener('keyup', e => e.stopPropagation());
    editor.addEventListener('input', () => {
      if (!update(active, editor.value, false)) close();
    });
    const done = document.createElement('button');
    done.type = 'button';
    done.textContent = 'Done';
    done.addEventListener('click', () => { update(active, editor.value, true); close(); });
    label.append(title, editor);
    panel.append(label, done);
    document.body.append(panel);
    position();
    // Synchronous focus inside pointerup is required on iOS; never defer it.
    editor.focus({preventScroll: true});
  };
  const cancel = () => { start = null; };
  document.addEventListener('pointerdown', down);
  document.addEventListener('pointerup', up);
  document.addEventListener('pointercancel', cancel);
  window.visualViewport?.addEventListener('resize', position);
  window.visualViewport?.addEventListener('scroll', position);
  return () => {
    document.removeEventListener('pointerdown', down);
    document.removeEventListener('pointerup', up);
    document.removeEventListener('pointercancel', cancel);
    window.visualViewport?.removeEventListener('resize', position);
    window.visualViewport?.removeEventListener('scroll', position);
    close();
  };
}
