// Canvas text fields need a real browser editor to open the mobile keyboard.
export function installMobileInput(open, update) {
  let start = null;
  let panel = null;
  let editor = null;
  let active = null;
  const close = () => {
    const previousEditor = editor;
    const previousPanel = panel;
    // Removing a focused element may synchronously emit blur.
    panel = editor = active = null;
    if (previousEditor) previousEditor.value = '';
    if (previousPanel) previousPanel.remove();
  };
  const finish = () => {
    if (!editor) return;
    const token = active, value = editor.value;
    close();
    update(token, value, true);
  };
  const position = () => {
    if (!panel) return;
    const view = window.visualViewport;
    panel.style.bottom = `${view ? Math.max(0, window.innerHeight - view.height - view.offsetTop) : 0}px`;
  };
  const down = e => {
    if (panel && e.isPrimary && !panel.contains(e.target)) finish();
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
    const fieldEditor = editor;
    const finishField = () => { if (editor === fieldEditor) finish(); };
    editor.addEventListener('keydown', e => {
      e.stopPropagation();
      if (e.key === 'Escape') finishField();
    });
    editor.addEventListener('keyup', e => e.stopPropagation());
    editor.addEventListener('blur', finishField);
    editor.addEventListener('input', () => {
      if (editor !== fieldEditor) return;
      if (!update(active, editor.value, false)) close();
    });
    const done = document.createElement('button');
    done.type = 'button';
    done.textContent = 'Done';
    done.addEventListener('click', finishField);
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
