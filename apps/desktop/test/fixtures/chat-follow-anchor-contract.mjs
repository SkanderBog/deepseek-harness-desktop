import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import process from 'node:process'
import { test } from 'node:test'
import { runInNewContext } from 'node:vm'

const source = readFileSync(process.argv[2], 'utf8')
const ScrollFollow = runInNewContext(`${source}\nScrollFollow`, {}, { timeout: 1000 })
const ATTRIBUTE = 'data-chat-following-tail'

function scrollport() {
  const attributes = new Set()
  const writes = []
  return {
    attributes,
    writes,
    hasAttribute: name => attributes.has(name),
    setAttribute(name, value) {
      writes.push(['set', name, value])
      attributes.add(name)
    },
    removeAttribute(name) {
      writes.push(['remove', name])
      attributes.delete(name)
    },
  }
}

function bind({ nested = true, port = scrollport() } = {}) {
  let attached = nested
  const element = {
    closest: selector => (attached && selector === '[data-conversation-scroll]' ? port : null),
  }
  const follow = new ScrollFollow()
  return {
    follow,
    port,
    detach: () => {
      attached = false
    },
    unbind: follow.bind(element),
  }
}

test('following mirrors the intent onto the enclosing scrollport', () => {
  const { follow, port } = bind()
  assert.equal(port.attributes.has(ATTRIBUTE), false)
  follow.setFollowing(true)
  assert.equal(port.attributes.has(ATTRIBUTE), true)
})

test('unchanged intent does not rewrite the mirror', () => {
  const { follow, port } = bind()
  follow.setFollowing(true)
  follow.setFollowing(true)
  follow.setFollowing(true)
  assert.deepEqual(port.writes, [['set', ATTRIBUTE, '']])
})

test('leaving the tail clears the mirror', () => {
  const { follow, port } = bind()
  follow.setFollowing(true)
  follow.setFollowing(false)
  assert.equal(port.attributes.has(ATTRIBUTE), false)
  assert.deepEqual(port.writes, [
    ['set', ATTRIBUTE, ''],
    ['remove', ATTRIBUTE],
  ])
})

test('releasing the scrollport clears the mirror', () => {
  const { follow, port, unbind } = bind()
  follow.setFollowing(true)
  unbind()
  assert.equal(port.attributes.has(ATTRIBUTE), false)
})

test('clearing works when the element is detached before release', () => {
  const { follow, port, detach, unbind } = bind()
  follow.setFollowing(true)
  assert.equal(port.attributes.has(ATTRIBUTE), true)
  detach()
  unbind()
  assert.equal(port.attributes.has(ATTRIBUTE), false)
})

test('without an enclosing conversation scrollport nothing is written', () => {
  const { follow, port } = bind({ nested: false })
  follow.setFollowing(true)
  assert.deepEqual(port.writes, [])
})

test('releasing a stale binding still drops its follower', () => {
  const port = scrollport()
  const element = {
    closest: selector => (selector === '[data-conversation-scroll]' ? port : null),
  }
  const stale = new ScrollFollow()
  const unbindStale = stale.bind(element)
  stale.setFollowing(true)
  const current = new ScrollFollow()
  current.bind(element)
  assert.equal(port.attributes.has(ATTRIBUTE), true)
  unbindStale()
  assert.equal(port.attributes.has(ATTRIBUTE), false)
  current.setFollowing(false)
  assert.equal(port.attributes.has(ATTRIBUTE), false)
})

test('one follower leaving the tail keeps the mirror while another follows', () => {
  const shared = scrollport()
  const first = bind({ port: shared })
  first.follow.setFollowing(true)
  const second = bind({ port: shared })
  second.follow.setFollowing(true)
  second.follow.setFollowing(false)
  assert.equal(shared.attributes.has(ATTRIBUTE), true)
  second.unbind()
  assert.equal(shared.attributes.has(ATTRIBUTE), true)
  first.unbind()
  assert.equal(shared.attributes.has(ATTRIBUTE), false)
})
