/**
 * The property of the isolated world's global that holds Neo's glue objects, by glue key. Only
 * code in the world can read it: page scripts run in the main world, whose global is another
 * object (worlds share the DOM, not their globals).
 */
export const WORLD_KEY = '__neo_injected__'
