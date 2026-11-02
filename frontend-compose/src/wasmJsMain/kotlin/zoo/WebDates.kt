@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package zoo

import kotlin.JsFun

@JsFun(
    """(value, locale, kind, offset) => { const raw = /[Z+-]\d|Z$/.test(value.slice(10)) ? value : value.replace(' ', 'T') + 'Z'; const date=new Date(raw); if(Number.isNaN(+date)) return value; if(offset) date.setDate(date.getDate()+offset); if(kind==='full')return date.toLocaleString(locale); return new Intl.DateTimeFormat(locale, kind==='notification' ? {month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',hourCycle:'h23'} : kind==='day' ? {year:'numeric',month:'long',day:'numeric',weekday:'short'} : {hour:'2-digit',minute:'2-digit',hourCycle:'h23'}).format(date); }"""
)
internal external fun webFormatDate(
    value: String,
    locale: String,
    kind: String,
    offset: Int,
): String

@JsFun("() => new Date().toISOString()") internal external fun webCurrentTime(): String
