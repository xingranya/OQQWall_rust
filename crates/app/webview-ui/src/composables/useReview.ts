import { computed, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { api } from '../api/client'
import { STAGE_LABELS, type PostDetail, type PostItem, type Stage } from '../api/types'

export function useReview() {
  const message = useMessage()
  const loading = ref(false)
  const detailLoading = ref(false)
  const actionLoading = ref(false)

  const stage = ref<Stage>('review_pending')
  const keyword = ref('')
  const posts = ref<PostItem[]>([])

  const selectedReviewIds = ref<string[]>([])

  const detail = ref<PostDetail | null>(null)
  const detailOpen = ref(false)
  const currentDetailId = ref<string | null>(null)

  const pendingCount = computed(() => posts.value.filter((item) => item.stage === 'review_pending').length)

  const filteredPosts = computed(() => {
    const q = keyword.value.trim().toLowerCase()
    if (!q) return posts.value
    return posts.value.filter((post) => {
      const fields = [
        post.post_id,
        post.review_id ?? '',
        post.group_id,
        post.sender_id ?? '',
        String(post.internal_code ?? ''),
        String(post.external_code ?? ''),
        STAGE_LABELS[post.stage] ?? post.stage,
        post.last_error ?? '',
        post.preview_text ?? '',
      ]
      return fields.join(' ').toLowerCase().includes(q)
    })
  })

  const selectableReviewIds = computed(() =>
    filteredPosts.value.map((item) => item.review_id).filter(Boolean) as string[],
  )

  const allSelected = computed(() => {
    if (selectableReviewIds.value.length === 0) return false
    return selectableReviewIds.value.every((id) => selectedReviewIds.value.includes(id))
  })

  async function loadPosts() {
    loading.value = true
    try {
      const result = await api<{ items: PostItem[] }>('/api/posts?stage=' + stage.value + '&limit=200')
      posts.value = result.items
      const reviewSet = new Set(result.items.map((item) => item.review_id).filter(Boolean) as string[])
      selectedReviewIds.value = selectedReviewIds.value.filter((id) => reviewSet.has(id))
    } catch (err) {
      message.error((err as Error).message)
    } finally {
      loading.value = false
    }
  }

  async function openDetail(postId: string) {
    currentDetailId.value = postId
    detailOpen.value = true
    detailLoading.value = true
    try {
      detail.value = await api<PostDetail>('/api/posts/' + postId)
    } catch (err) {
      detail.value = null
      message.error((err as Error).message)
    } finally {
      detailLoading.value = false
    }
  }

  async function refreshDetail() {
    if (!currentDetailId.value) return
    detailLoading.value = true
    try {
      detail.value = await api<PostDetail>('/api/posts/' + currentDetailId.value)
    } catch (err) {
      message.error((err as Error).message)
    } finally {
      detailLoading.value = false
    }
  }

  function toggleSelectAll() {
    if (allSelected.value) {
      selectedReviewIds.value = selectedReviewIds.value.filter((id) => !selectableReviewIds.value.includes(id))
      return
    }
    const set = new Set([...selectedReviewIds.value, ...selectableReviewIds.value])
    selectedReviewIds.value = [...set]
  }

  function toggleOneSelection(reviewId: string, checked: boolean) {
    if (checked) {
      if (!selectedReviewIds.value.includes(reviewId)) {
        selectedReviewIds.value = [...selectedReviewIds.value, reviewId]
      }
    } else {
      selectedReviewIds.value = selectedReviewIds.value.filter((id) => id !== reviewId)
    }
  }

  return {
    loading,
    detailLoading,
    actionLoading,
    stage,
    keyword,
    posts,
    filteredPosts,
    pendingCount,
    selectedReviewIds,
    detail,
    detailOpen,
    allSelected,
    loadPosts,
    openDetail,
    refreshDetail,
    toggleSelectAll,
    toggleOneSelection,
  }
}
