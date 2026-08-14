package com.lanpulse.mobile.android

import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReferenceArray

internal class SpscRing<T>(private val capacity: Int) {
    private val mask = capacity - 1
    private val slots = AtomicReferenceArray<T?>(capacity)
    private val publishedProducerIndex = AtomicLong(0)
    private val publishedConsumerIndex = AtomicLong(0)
    private var producerIndex = 0L
    private var consumerIndex = 0L

    init {
        require(capacity >= 2 && capacity.countOneBits() == 1)
    }

    fun offer(value: T): Boolean {
        val index = producerIndex
        if (index - publishedConsumerIndex.get() >= capacity) return false
        slots.lazySet((index and mask.toLong()).toInt(), value)
        producerIndex = index + 1
        publishedProducerIndex.lazySet(producerIndex)
        return true
    }

    fun poll(): T? {
        val index = consumerIndex
        if (index >= publishedProducerIndex.get()) return null
        val slot = (index and mask.toLong()).toInt()
        val value = slots.get(slot) ?: return null
        slots.lazySet(slot, null)
        consumerIndex = index + 1
        publishedConsumerIndex.lazySet(consumerIndex)
        return value
    }
}
