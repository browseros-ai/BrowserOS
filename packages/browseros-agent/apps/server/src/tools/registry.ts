import {
  create_bookmark,
  get_bookmarks,
  move_bookmark,
  remove_bookmark,
  search_bookmarks,
  update_bookmark,
} from './bookmarks'
import { browseros_info } from './browseros-info'
import { get_console_logs } from './console'
import { get_dom, search_dom } from './dom'
import {
  filesystem_bash,
  filesystem_edit,
  filesystem_find,
  filesystem_grep,
  filesystem_ls,
  filesystem_read,
  filesystem_write,
  fs_edit,
  fs_list,
  fs_read,
  fs_write,
  shell_execute,
} from './filesystem-registry'
import {
  delete_history_range,
  delete_history_url,
  get_recent_history,
  search_history,
} from './history'
import {
  check,
  clear,
  click,
  click_at,
  drag,
  drag_at,
  fill,
  focus,
  handle_dialog,
  hover,
  hover_at,
  press_key,
  scroll,
  select_option,
  type_at,
  uncheck,
  upload_file,
} from './input'
import {
  close_page,
  get_active_page,
  list_pages,
  move_page,
  navigate_page,
  new_hidden_page,
  new_page,
  show_page,
  // biome-ignore lint/correctness/noUnusedImports: temporarily disabled
  wait_for,
} from './navigation'
import { suggest_app_connection, suggest_schedule } from './nudges'
import { download_file, save_pdf, save_screenshot } from './page-actions'
import {
  evaluate_script,
  get_page_content,
  get_page_links,
  take_enhanced_snapshot,
  take_screenshot,
  take_snapshot,
} from './snapshot'
import {
  close_tab_group,
  group_tabs,
  list_tab_groups,
  ungroup_tabs,
  update_tab_group,
} from './tab-groups'
import { createRegistry } from './tool-registry'
import {
  activate_window,
  close_window,
  create_hidden_window,
  create_window,
  list_windows,
  set_window_visibility,
} from './windows'

export const registry = createRegistry([
  // Navigation (8)
  get_active_page,
  list_pages,
  navigate_page,
  new_page,
  new_hidden_page,
  show_page,
  move_page,
  close_page,
  // wait_for, // temporarily disabled

  // Observation (9)
  take_snapshot,
  take_enhanced_snapshot,
  get_page_content,
  get_page_links,
  get_dom,
  search_dom,
  take_screenshot,
  evaluate_script,
  get_console_logs,

  // Input (17)
  click,
  click_at,
  hover,
  hover_at,
  type_at,
  drag_at,
  focus,
  clear,
  fill,
  check,
  uncheck,
  upload_file,
  press_key,
  drag,
  scroll,
  handle_dialog,
  select_option,

  // Page Actions (3)
  save_pdf,
  save_screenshot,
  download_file,

  // Windows (6)
  list_windows,
  create_window,
  create_hidden_window,
  close_window,
  activate_window,
  set_window_visibility,

  // Bookmarks (6)
  get_bookmarks,
  create_bookmark,
  remove_bookmark,
  update_bookmark,
  move_bookmark,
  search_bookmarks,

  // History (4)
  search_history,
  get_recent_history,
  delete_history_url,
  delete_history_range,

  // Tab Groups (5)
  list_tab_groups,
  group_tabs,
  update_tab_group,
  ungroup_tabs,
  close_tab_group,

  // Info (1)
  browseros_info,

  // Filesystem (7)
  filesystem_bash,
  filesystem_ls,
  filesystem_read,
  filesystem_write,
  filesystem_edit,
  filesystem_find,
  filesystem_grep,

  // Filesystem aliases (5)
  fs_read,
  fs_write,
  fs_list,
  shell_execute,
  fs_edit,

  // Nudges (2)
  suggest_schedule,
  suggest_app_connection,
])
