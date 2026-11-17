/** Marks diagnostic assertions without changing behavior assertions or evaluated page values.
 * This is the only deliberate transform beyond upstream's compiler; the inventory exposes it. */
module.exports = ({ types: t }) => ({
  pre() {
    this.file.metadata.messageAssertions = []
  },
  visitor: {
    CallExpression(path, state) {
      const node = path.node
      if (!t.isIdentifier(node.callee, { name: 'expect' })) return
      let outer = path
      while (
        outer.parentPath?.isMemberExpression() ||
        outer.parentPath?.isCallExpression()
      )
        outer = outer.parentPath
      const matcherCall = outer.node
      if (
        !t.isCallExpression(matcherCall) ||
        !t.isMemberExpression(matcherCall.callee)
      )
        return
      const matcher = matcherCall.callee.property.name
      let member = matcherCall.callee
      const modifiers = []
      while (t.isMemberExpression(member.object)) {
        member = member.object
        modifiers.push(member.property.name)
      }
      if (modifiers.includes('not')) return
      let kind
      const received = node.arguments[0]
      if (
        t.isMemberExpression(received) &&
        !received.computed &&
        t.isIdentifier(received.property, { name: 'message' }) &&
        ['toBe', 'toEqual', 'toContain', 'toMatch', 'toStrictEqual'].includes(
          matcher,
        )
      )
        kind = 'message'
      else if (
        ['toThrow', 'toThrowError'].includes(matcher) &&
        (t.isStringLiteral(matcherCall.arguments[0]) ||
          t.isRegExpLiteral(matcherCall.arguments[0]))
      )
        kind = 'throw'
      if (!kind) return
      const meta = {
        line: node.loc.start.line,
        column: node.loc.start.column,
        matcher,
        kind,
        rejects: modifiers.includes('rejects'),
      }
      state.file.metadata.messageAssertions.push(meta)
      node.callee = t.memberExpression(
        t.identifier('__harness'),
        t.identifier('messageExpect'),
      )
      node.arguments = [
        t.valueToNode(meta),
        kind === 'message' ? received.object : received,
        ...node.arguments.slice(1),
      ]
    },
  },
})
