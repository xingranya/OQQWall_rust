import { ref } from 'vue'
import { api } from '../api/client'
import type { MeResponse } from '../api/types'

const authed = ref(false)
const me = ref<MeResponse | null>(null)
const loginLoading = ref(false)

export function useAuth() {
  async function checkSession() {
    try {
      const result = await api<MeResponse>('/auth/me', { method: 'GET' })
      authed.value = true
      me.value = result
    } catch {
      authed.value = false
      me.value = null
    }
  }

  async function login(username: string, passwordHash: string) {
    loginLoading.value = true
    try {
      await api('/auth/login', {
        method: 'POST',
        body: JSON.stringify({
          username: username.trim(),
          password: passwordHash,
        }),
      })
      await checkSession()
      return true
    } catch (e) {
      throw e
    } finally {
      loginLoading.value = false
    }
  }

  async function logout() {
    try {
      await api('/auth/logout', { method: 'POST' })
    } finally {
      authed.value = false
      me.value = null
    }
  }

  return {
    authed,
    me,
    loginLoading,
    checkSession,
    login,
    logout,
  }
}
