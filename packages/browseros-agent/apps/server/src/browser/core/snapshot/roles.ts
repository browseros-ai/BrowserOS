export const INTERACTIVE_ROLES: ReadonlySet<string> = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'textarea',
  'checkbox',
  'radio',
  'combobox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'tab',
  'switch',
  'slider',
  'spinbutton',
  'option',
  'treeitem',
  'listbox',
  'DisclosureTriangle',
])

export const SKIP_ROLES: ReadonlySet<string> = new Set([
  'none',
  'presentation',
  'LineBreak',
  'InlineTextBox',
  'StaticText',
  'text',
])

export const ROOT_ROLES: ReadonlySet<string> = new Set([
  'RootWebArea',
  'WebArea',
])

export const VALUE_ROLES: ReadonlySet<string> = new Set([
  'textbox',
  'searchbox',
  'textarea',
  'combobox',
  'spinbutton',
])
