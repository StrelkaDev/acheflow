// A single real editor follows the focused Blitz field.
export function installTextInput(open, readRect, edit) {
  let press = null;
  let active = null;
  let frame = 0;
  let keyboardFrame = 0;

  function close() {
    const state = active;
    active = null;
    if (!state) return;
    cancelAnimationFrame(frame);
    state.input.remove();
    state.input.value = '';
  }

  function publish(state, value) {
    if (value === state.published) return true;
    state.published = value;
    return edit('input', value);
  }

  function finish(action) {
    const state = active;
    if (!state) return;
    const value = action === 'escape' ? state.initial
      : state.composing ? state.published : state.input.value;
    close();
    if (publish(state, value)) edit(action, value);
    if (action !== 'blur') state.canvas.focus({preventScroll: true});
    if (action === 'tab' || action === 'backtab') openFocused(state.canvas);
  }

  function position(rect) {
    const state = active;
    if (!state) return;
    const canvas = state.canvas.getBoundingClientRect();
    const viewport = window.visualViewport;
    const xScale = canvas.width / state.canvas.clientWidth;
    const yScale = canvas.height / state.canvas.clientHeight;
    let width = rect.width * xScale;
    let height = rect.height * yScale;
    let left = canvas.left + rect.x * xScale;
    let top = canvas.top + rect.y * yScale;
    // On a zoomed page or above a software keyboard, keep the whole editor reachable.
    if (viewport && viewport.width > 0 && viewport.height > 0) {
      width = Math.min(width, viewport.width);
      height = Math.min(height, viewport.height);
      left = Math.max(viewport.offsetLeft, Math.min(left, viewport.offsetLeft + viewport.width - width));
      top = Math.max(viewport.offsetTop, Math.min(top, viewport.offsetTop + viewport.height - height));
    }
    Object.assign(state.input.style, {
      left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px`,
    });
  }

  function track() {
    if (!active) return;
    const rect = readRect();
    if (!rect) {
      finish('blur');
      return;
    }
    position(JSON.parse(rect));
    frame = requestAnimationFrame(track);
  }

  function openEditor(canvas, data) {
    if (!data || active) return;
    const field = JSON.parse(data);
    const input = document.createElement(field.multiline ? 'textarea' : 'input');
    input.className = `acheflow-text-editor${field.code ? ' acheflow-code-editor' : ''}`;
    if (!field.multiline) input.type = field.secret ? 'password' : 'text';
    if (field.code) input.wrap = 'off';
    input.setAttribute('aria-label', field.label);
    input.placeholder = field.placeholder;
    if (field.padding) {
      const {top, right, bottom, left} = field.padding;
      input.style.padding = `${top}px ${right}px ${bottom}px ${left}px`;
    }
    input.inputMode = field.input_mode;
    if (field.max_length != null) input.maxLength = field.max_length;
    input.autocomplete = 'off';
    input.autocapitalize = 'off';
    input.spellcheck = false;
    input.value = field.value;
    const state = {input, canvas, initial: field.value, published: field.value, composing: false};
    active = state;
    const send = () => {
      if (active === state && !publish(state, input.value)) close();
    };
    input.addEventListener('compositionstart', () => { state.composing = true; });
    input.addEventListener('input', event => { if (!state.composing && !event.isComposing) send(); });
    input.addEventListener('compositionend', () => {
      state.composing = false;
      send();
    });
    input.addEventListener('blur', () => { if (active === state) finish('blur'); });
    input.addEventListener('keydown', event => {
      event.stopPropagation();
      if (state.composing || event.isComposing || event.keyCode === 229 || event.ctrlKey || event.metaKey || event.altKey) return;
      const action = {
        Escape: 'escape',
        Tab: event.shiftKey ? 'backtab' : 'tab',
        Enter: field.multiline ? null : 'enter',
      }[event.key];
      if (action) {
        event.preventDefault();
        finish(action);
      }
    });
    input.addEventListener('keyup', event => event.stopPropagation());
    document.body.append(input);
    position(field.rect);
    input.focus({preventScroll: true}); // Synchronous for iOS pointer activation.
    input.setSelectionRange(field.start, field.end);
    frame = requestAnimationFrame(track);
  }

  function openFocused(canvas) {
    openEditor(canvas, open(0, 0, 0, 0));
  }

  function openAt(event) {
    const canvas = event.target;
    const rect = canvas.getBoundingClientRect();
    openEditor(canvas, open(event.clientX - rect.left, event.clientY - rect.top, rect.width, rect.height));
  }

  const down = event => {
    // Let the browser finish IME naturally when the pointer changes focus.
    if (active && !active.composing && event.isPrimary && event.target !== active.input) finish('blur');
    press = event.target.tagName === 'CANVAS' && event.isPrimary && event.button === 0
      ? {id: event.pointerId, x: event.clientX, y: event.clientY} : null;
  };
  const up = event => {
    const start = press;
    press = null;
    if (!start || start.id !== event.pointerId || event.target.tagName !== 'CANVAS') return;
    if (event.pointerType !== 'mouse' && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 12) return;
    if (active) finish('blur');
    openAt(event);
  };
  const cancel = () => { press = null; };
  const keydown = event => {
    if (event.key === 'Tab' && !event.ctrlKey && !event.metaKey && !event.altKey && event.target.tagName === 'CANVAS') {
      cancelAnimationFrame(keyboardFrame);
      keyboardFrame = requestAnimationFrame(() => openFocused(event.target));
    }
  };
  const context = event => {
    if (!active && event.target.tagName === 'CANVAS') openAt(event);
  };

  document.addEventListener('pointerdown', down, true);
  document.addEventListener('pointerup', up);
  document.addEventListener('pointercancel', cancel);
  document.addEventListener('keydown', keydown);
  document.addEventListener('contextmenu', context);
  return () => {
    document.removeEventListener('pointerdown', down, true);
    document.removeEventListener('pointerup', up);
    document.removeEventListener('pointercancel', cancel);
    document.removeEventListener('keydown', keydown);
    document.removeEventListener('contextmenu', context);
    cancelAnimationFrame(keyboardFrame);
    close();
  };
}
