// Layout-only enhancement of the same static prototype content; no backend actions.
const narrow = matchMedia('(max-width: 1023px)');
const phone = matchMedia('(max-width: 639px)');
const make = (tag, className, text) => {
  const el = document.createElement(tag);
  el.className = className;
  if (text) el.textContent = text;
  return el;
};

const sidebar = document.querySelector('.sidebar');
if (sidebar) {
  const trigger = make('button', 'btn drawer-trigger', '☰ 菜单');
  trigger.setAttribute('aria-haspopup', 'dialog');
  const dialog = make('dialog', 'navigation-dialog');
  dialog.setAttribute('aria-label', '工作空间导航');
  const close = make('button', 'btn', '关闭菜单');
  const original = document.createComment('desktop navigation');
  sidebar.before(original);
  const topnav = document.querySelector('.topnav');
  const topOriginal = document.createComment('desktop top navigation');
  topnav?.before(topOriginal);
  dialog.append(close);
  document.body.append(dialog);
  document.querySelector('.top').prepend(trigger);
  trigger.addEventListener('click', () => dialog.showModal());
  close.addEventListener('click', () => dialog.close());
  dialog.addEventListener('click', e => { if (e.target === dialog) dialog.close(); });
  const update = () => {
    if (narrow.matches) {
      dialog.append(sidebar);
      if (topnav) dialog.append(topnav);
    } else {
      dialog.close();
      original.after(sidebar);
      if (topnav) topOriginal.after(topnav);
    }
  };
  narrow.addEventListener('change', update);
  update();
}

// Keep filters and pagination outside this bounded scrolling owner (§7.3).
document.querySelectorAll('table').forEach(table => {
  const scroll = make('div', 'table-scroll');
  scroll.tabIndex = 0;
  scroll.setAttribute('role', 'region');
  scroll.setAttribute('aria-label', '数据表格，可横向滚动查看全部列');
  table.before(scroll);
  scroll.append(table);
  const caption = make('div', 'table-caption', '左右滚动查看全部列 · 首列固定');
  scroll.before(caption);
  const homes = [...table.querySelectorAll('.callout')].map(marker => {
    const home = document.createComment('table marker');
    marker.before(home);
    return { marker, home };
  });
  const update = () => homes.forEach(({ marker, home }) => narrow.matches ? caption.append(marker) : home.after(marker));
  narrow.addEventListener('change', update);
  update();
});

// Preserve original numbered markers on the visible tab labels while a pane is hidden.
document.querySelectorAll('.editor-layout, .threecol').forEach((group, index) => {
  const panes = [...group.children];
  if (panes.length < 2) return;
  group.classList.add('pane-group');
  const tabs = make('div', 'pane-tabs');
  tabs.setAttribute('role', 'tablist');
  tabs.setAttribute('aria-label', '编辑器面板');
  group.before(tabs);
  const markers = [];
  const buttons = panes.map((pane, n) => {
    const title = pane.matches('.canvas') ? '画布 / 节点' : pane.matches('.nodepanel') ? '节点库' :
      pane.querySelector('h2,h3')?.textContent.replace(/[①-⑳]/g, '').trim() || '问题 / 详情';
    const button = make('button', 'btn pane-tab', title);
    button.id = `pane-tab-${index}-${n}`;
    pane.id = `pane-${index}-${n}`;
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', pane.id);
    pane.setAttribute('aria-labelledby', button.id);
    tabs.append(button);
    pane.querySelectorAll('.callout').forEach(marker => {
      const home = document.createComment('numbered marker');
      marker.before(home);
      markers.push({ marker, home, button });
    });
    button.addEventListener('click', () => select(n));
    button.addEventListener('keydown', e => {
      let next;
      if (e.key === 'ArrowRight') next = (n + 1) % panes.length;
      if (e.key === 'ArrowLeft') next = (n + panes.length - 1) % panes.length;
      if (e.key === 'Home') next = 0;
      if (e.key === 'End') next = panes.length - 1;
      if (next !== undefined) { e.preventDefault(); select(next); buttons[next].focus(); }
    });
    return button;
  });
  let selected = Math.max(0, panes.findIndex(p => p.matches('.canvas')));
  function select(n) {
    selected = n;
    panes.forEach((pane, i) => {
      pane.classList.toggle('pane-inactive', i !== n);
      if (narrow.matches) pane.setAttribute('role', 'tabpanel');
      else pane.removeAttribute('role');
      buttons[i].setAttribute('aria-selected', String(i === n));
      buttons[i].tabIndex = i === n ? 0 : -1;
    });
  }
  const update = () => {
    markers.forEach(({ marker, home, button }) => narrow.matches ? button.append(marker) : home.after(marker));
    select(selected);
  };
  narrow.addEventListener('change', update);
  update();
});

// Reuse the existing node/job items as a vertical list, retaining statuses and actions.
document.querySelectorAll('.canvas').forEach(canvas => {
  const surface = make('div', 'graph-surface');
  surface.append(...canvas.childNodes);
  canvas.append(surface);
  const note = make('p', 'phone-graph-note', '只读节点列表 · 在桌面端编辑');
  canvas.prepend(note);
  if (!canvas.closest('.pane-group')) {
    const markers = make('div', 'pad-graph-markers', '画布标注');
    canvas.before(markers);
    const homes = [...canvas.querySelectorAll('.callout')].map(marker => {
      const home = document.createComment('graph marker');
      marker.before(home);
      return { marker, home };
    });
    const updateMarkers = () => homes.forEach(({ marker, home }) =>
      narrow.matches && !phone.matches ? markers.append(marker) : home.after(marker));
    narrow.addEventListener('change', updateMarkers);
    phone.addEventListener('change', updateMarkers);
    updateMarkers();
  }
  const nodes = [...canvas.querySelectorAll('.node')];
  nodes.forEach((node, i) => {
    node.tabIndex = 0;
    node.setAttribute('aria-label', node.textContent.trim().replace(/\s+/g, ' '));
    const selectNode = () => {
      nodes.forEach(n => n.classList.toggle('keyboard-selected', n === node));
      const group = canvas.closest('.pane-group');
      const detail = (group && [...group.children].find(p => p !== canvas && !p.matches('.nodepanel'))) || canvas.closest('.editor-frame')?.querySelector('.logtree');
      if (detail && narrow.matches) document.querySelector(`[aria-controls="${detail.id}"]`)?.click();
      else if (detail) { detail.tabIndex = -1; detail.focus(); }
    };
    node.addEventListener('keydown', e => {
      const delta = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
      if (delta) { e.preventDefault(); nodes[(i + delta + nodes.length) % nodes.length].focus(); }
      if (e.key === 'Enter') { e.preventDefault(); selectNode(); }
    });
  });
  const update = () => {
    canvas.setAttribute('role', phone.matches ? 'list' : 'region');
    canvas.setAttribute('aria-label', phone.matches ? '只读节点与状态列表' : '节点画布');
    nodes.forEach(node => node.setAttribute('role', phone.matches ? 'listitem' : 'group'));
  };
  phone.addEventListener('change', update);
  update();
});

// Static action text remains static, but is keyboard reachable like the original buttons.
document.querySelectorAll('.actions, .actions span, .menu > div, .navitem, .topnav > span, .media-card, .entry, .folder').forEach(el => {
  el.tabIndex = 0;
});
const updateLabel = () => document.querySelectorAll('.review-note > span:last-child').forEach(el => {
  el.textContent = `浅色主题 · ${innerWidth} × ${innerHeight} · ${phone.matches ? 'Phone' : narrow.matches ? 'Pad' : 'Desktop'}`;
});
window.addEventListener('resize', updateLabel);
updateLabel();
