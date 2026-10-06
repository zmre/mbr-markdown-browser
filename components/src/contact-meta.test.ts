import { describe, it, expect } from 'vitest'
import { isCardKey, isContactType } from './contact-meta.js'

describe('UNIT contact-meta', () => {
  it('recognises exactly the contact types', () => {
    expect(isContactType('person')).toBe(true)
    expect(isContactType('organization')).toBe(true)
    for (const t of ['Person', 'organisation', 'event', undefined, null, 1]) {
      expect(isContactType(t)).toBe(false)
    }
  })

  it('hides card fields and their flattened variants on contact pages', () => {
    for (const key of [
      'emails',
      'emails.work',
      'phones',
      'urls.homepage',
      'social.linkedin',
      'im.signal',
      'addresses.home.city',
      'dates.birthday',
      'dates.anniversary',
      'first_name',
      'middle_name',
      'last_name',
      'prefix',
      'suffix',
      'company',
      'department',
      'job_title',
    ]) {
      expect(isCardKey('person', key), key).toBe(true)
      expect(isCardKey('organization', key), key).toBe(true)
    }
  })

  it('keeps everything else on contact pages', () => {
    for (const key of ['title', 'tags', 'aliases', 'born', 'gender', 'emailsx', 'companyName']) {
      expect(isCardKey('person', key), key).toBe(false)
    }
  })

  it('leaves other note types untouched', () => {
    expect(isCardKey('event', 'emails.work')).toBe(false)
    expect(isCardKey(undefined, 'company')).toBe(false)
  })
})
