/*
** $Id: ltablib.c,v 1.93.1.1 2017/04/19 17:20:42 roberto Exp $
** Library for Table Manipulation
** See Copyright Notice in lua.h
*/

#define ltablib_c
#define LUA_LIB

#if defined(__wasi__)
#include <stdlib.h>
#endif

#include "lprefix.h"


#include <limits.h>
#include <stddef.h>
#include <string.h>

#include "lua.h"

#include "lauxlib.h"
#include "lualib.h"


/*
** Operations that an object must define to mimic a table
** (some functions only need some of them)
*/
#define TAB_R	1			/* read */
#define TAB_W	2			/* write */
#define TAB_L	4			/* length */
#define TAB_RW	(TAB_R | TAB_W)		/* read/write */


#define aux_getn(L,n,w)	(checktab(L, n, (w) | TAB_L), luaL_len(L, n))


static int checkfield (lua_State *L, const char *key, int n) {
  lua_pushstring(L, key);
  return (lua_rawget(L, -n) != LUA_TNIL);
}


/*
** Check that 'arg' either is a table or can behave like one (that is,
** has a metatable with the required metamethods)
*/
static void checktab (lua_State *L, int arg, int what) {
  if (lua_type(L, arg) != LUA_TTABLE) {  /* is it not a table? */
    int n = 1;  /* number of elements to pop */
    if (lua_getmetatable(L, arg) &&  /* must have metatable */
        (!(what & TAB_R) || checkfield(L, "__index", ++n)) &&
        (!(what & TAB_W) || checkfield(L, "__newindex", ++n)) &&
        (!(what & TAB_L) || checkfield(L, "__len", ++n))) {
      lua_pop(L, n);  /* pop metatable and tested metamethods */
    }
    else
      luaL_checktype(L, arg, LUA_TTABLE);  /* force an error */
  }
}


#if defined(LUA_COMPAT_MAXN)
static int maxn (lua_State *L) {
  lua_Number max = 0;
  luaL_checktype(L, 1, LUA_TTABLE);
  lua_pushnil(L);  /* first key */
  while (lua_next(L, 1)) {
    lua_pop(L, 1);  /* remove value */
    if (lua_type(L, -1) == LUA_TNUMBER) {
      lua_Number v = lua_tonumber(L, -1);
      if (v > max) max = v;
    }
  }
  lua_pushnumber(L, max);
  return 1;
}
#endif


static int tinsert (lua_State *L) {
  lua_Integer e = aux_getn(L, 1, TAB_RW) + 1;  /* first empty element */
  lua_Integer pos;  /* where to insert new element */
  switch (lua_gettop(L)) {
    case 2: {  /* called with only 2 arguments */
      pos = e;  /* insert new element at the end */
      break;
    }
    case 3: {
      lua_Integer i;
      pos = luaL_checkinteger(L, 2);  /* 2nd argument is the position */
      luaL_argcheck(L, 1 <= pos && pos <= e, 2, "position out of bounds");
      for (i = e; i > pos; i--) {  /* move up elements */
        lua_geti(L, 1, i - 1);
        lua_seti(L, 1, i);  /* t[i] = t[i - 1] */
      }
      break;
    }
    default: {
      return luaL_error(L, "wrong number of arguments to 'insert'");
    }
  }
  lua_seti(L, 1, pos);  /* t[pos] = v */
  return 0;
}


static int tremove (lua_State *L) {
  lua_Integer size = aux_getn(L, 1, TAB_RW);
  lua_Integer pos = luaL_optinteger(L, 2, size);
  if (pos != size)  /* validate 'pos' if given */
    luaL_argcheck(L, 1 <= pos && pos <= size + 1, 1, "position out of bounds");
  lua_geti(L, 1, pos);  /* result = t[pos] */
  for ( ; pos < size; pos++) {
    lua_geti(L, 1, pos + 1);
    lua_seti(L, 1, pos);  /* t[pos] = t[pos + 1] */
  }
  lua_pushnil(L);
  lua_seti(L, 1, pos);  /* t[pos] = nil */
  return 1;
}


/*
** Copy elements (1[f], ..., 1[e]) into (tt[t], tt[t+1], ...). Whenever
** possible, copy in increasing order, which is better for rehashing.
** "possible" means destination after original range, or smaller
** than origin, or copying to another table.
*/
static int tmove (lua_State *L) {
  lua_Integer f = luaL_checkinteger(L, 2);
  lua_Integer e = luaL_checkinteger(L, 3);
  lua_Integer t = luaL_checkinteger(L, 4);
  int tt = !lua_isnoneornil(L, 5) ? 5 : 1;  /* destination table */
  checktab(L, 1, TAB_R);
  checktab(L, tt, TAB_W);
  if (e >= f) {  /* otherwise, nothing to move */
    lua_Integer n, i;
    luaL_argcheck(L, f > 0 || e < LUA_MAXINTEGER + f, 3,
                  "too many elements to move");
    n = e - f + 1;  /* number of elements to move */
    luaL_argcheck(L, t <= LUA_MAXINTEGER - n + 1, 4,
                  "destination wrap around");
    if (t > e || t <= f || (tt != 1 && !lua_compare(L, 1, tt, LUA_OPEQ))) {
      for (i = 0; i < n; i++) {
        lua_geti(L, 1, f + i);
        lua_seti(L, tt, t + i);
      }
    }
    else {
      for (i = n - 1; i >= 0; i--) {
        lua_geti(L, 1, f + i);
        lua_seti(L, tt, t + i);
      }
    }
  }
  lua_pushvalue(L, tt);  /* return destination table */
  return 1;
}


static void addfield (lua_State *L, luaL_Buffer *b, lua_Integer i) {
  lua_geti(L, 1, i);
  if (!lua_isstring(L, -1))
    luaL_error(L, "invalid value (%s) at index %d in table for 'concat'",
                  luaL_typename(L, -1), i);
  luaL_addvalue(b);
}


static int tconcat (lua_State *L) {
  luaL_Buffer b;
  lua_Integer last = aux_getn(L, 1, TAB_R);
  size_t lsep;
  const char *sep = luaL_optlstring(L, 2, "", &lsep);
  lua_Integer i = luaL_optinteger(L, 3, 1);
  last = luaL_optinteger(L, 4, last);
  luaL_buffinit(L, &b);
  for (; i < last; i++) {
    addfield(L, &b, i);
    luaL_addlstring(&b, sep, lsep);
  }
  if (i == last)  /* add last value (if interval was not empty) */
    addfield(L, &b, i);
  luaL_pushresult(&b);
  return 1;
}


/*
** {======================================================
** Pack/unpack
** =======================================================
*/

static int pack (lua_State *L) {
  int i;
  int n = lua_gettop(L);  /* number of elements to pack */
  lua_createtable(L, n, 1);  /* create result table */
  lua_insert(L, 1);  /* put it at index 1 */
  for (i = n; i >= 1; i--)  /* assign elements */
    lua_seti(L, 1, i);
  lua_pushinteger(L, n);
  lua_setfield(L, 1, "n");  /* t.n = number of elements */
  return 1;  /* return table */
}


static int unpack (lua_State *L) {
  lua_Unsigned n;
  lua_Integer i = luaL_optinteger(L, 2, 1);
  lua_Integer e = luaL_opt(L, luaL_checkinteger, 3, luaL_len(L, 1));
  if (i > e) return 0;  /* empty range */
  n = (lua_Unsigned)e - i;  /* number of elements minus 1 (avoid overflows) */
  if (n >= (unsigned int)INT_MAX  || !lua_checkstack(L, (int)(++n)))
    return luaL_error(L, "too many results to unpack");
  for (; i < e; i++) {  /* push arg[i..e - 1] (to avoid overflows) */
    lua_geti(L, 1, i);
  }
  lua_geti(L, 1, e);  /* push last element */
  return (int)n;
}

/* }====================================================== */



/*
** {======================================================
** STA64: stable merge sort (issue #1300), in place of Lua's quicksort,
** which took its pivot from clock() and time() after an unbalanced
** partition. Elements that compare equal keep their order, and nothing
** depends on the clock. The elements are copied into a buffer table, sorted
** there and written back, so the table is left as it was if the comparator
** raises an error. A comparator that is not a strict weak order yields some
** permutation of the elements, never an error of its own.
** =======================================================
*/


/* runs of this many elements are first sorted by insertion */
#define SORTRUN		8


/*
** Return true iff value at (absolute) stack index 'a' is less than the
** value at index 'b' (according to the order of the sort).
*/
static int sort_less (lua_State *L, int a, int b) {
  if (lua_isnil(L, 2))  /* no function? */
    return lua_compare(L, a, b, LUA_OPLT);  /* a < b */
  else {  /* function */
    int res;
    lua_pushvalue(L, 2);    /* push function */
    lua_pushvalue(L, a);
    lua_pushvalue(L, b);
    lua_call(L, 2, 1);      /* call function */
    res = lua_toboolean(L, -1);  /* get result */
    lua_pop(L, 1);          /* pop result */
    return res;
  }
}


/* stable insertion sort of buf[lo .. up] (buffer at stack index 'buf') */
static void sort_run (lua_State *L, int buf, lua_Integer lo, lua_Integer up) {
  lua_Integer i, j;
  for (i = lo + 1; i <= up; i++) {
    lua_rawgeti(L, buf, i);  /* x = buf[i] */
    for (j = i - 1; j >= lo; j--) {
      lua_rawgeti(L, buf, j);
      if (!sort_less(L, lua_gettop(L) - 1, lua_gettop(L))) {  /* x >= buf[j]? */
        lua_pop(L, 1);
        break;
      }
      lua_rawseti(L, buf, j + 1);  /* buf[j + 1] = buf[j] */
    }
    lua_rawseti(L, buf, j + 1);  /* buf[j + 1] = x */
  }
}


/* merge src[lo .. mid - 1] and src[mid .. hi - 1] into dst[lo .. hi - 1];
   of two equal elements the one from the left run goes first */
static void sort_merge (lua_State *L, int src, int dst, lua_Integer lo,
                        lua_Integer mid, lua_Integer hi) {
  lua_Integer a = lo, b = mid, k = lo;
  while (a < mid && b < hi) {
    lua_rawgeti(L, src, a);
    lua_rawgeti(L, src, b);
    if (sort_less(L, lua_gettop(L), lua_gettop(L) - 1)) {  /* src[b] < src[a]? */
      lua_rawseti(L, dst, k++);  /* take src[b] */
      lua_pop(L, 1);
      b++;
    }
    else {
      lua_pop(L, 1);
      lua_rawseti(L, dst, k++);  /* take src[a] */
      a++;
    }
  }
  for (; a < mid; a++) {
    lua_rawgeti(L, src, a);
    lua_rawseti(L, dst, k++);
  }
  for (; b < hi; b++) {
    lua_rawgeti(L, src, b);
    lua_rawseti(L, dst, k++);
  }
}


static int sort (lua_State *L) {
  lua_Integer n = aux_getn(L, 1, TAB_RW);
  if (n > 1) {  /* non-trivial interval? */
    lua_Integer i, lo, width;
    int src = 3, dst = 4;
    luaL_argcheck(L, n < INT_MAX, 1, "array too big");
    if (!lua_isnoneornil(L, 2))  /* is there a 2nd argument? */
      luaL_checktype(L, 2, LUA_TFUNCTION);  /* must be a function */
    lua_settop(L, 2);  /* make sure there are two arguments */
    lua_createtable(L, (int)n, 0);  /* 3: buffer */
    lua_createtable(L, (int)n, 0);  /* 4: second buffer */
    for (i = 1; i <= n; i++) {
      lua_geti(L, 1, i);
      lua_rawseti(L, src, i);
    }
    for (lo = 1; lo <= n; lo += SORTRUN)
      sort_run(L, src, lo, (n - lo < SORTRUN) ? n : lo + SORTRUN - 1);
    for (width = SORTRUN; width < n; width *= 2) {
      int tmp;
      for (lo = 1; lo <= n; lo += 2 * width) {
        lua_Integer mid = (n - lo < width) ? n + 1 : lo + width;
        lua_Integer hi = (n - lo < 2 * width) ? n + 1 : lo + 2 * width;
        sort_merge(L, src, dst, lo, mid, hi);
      }
      tmp = src; src = dst; dst = tmp;
    }
    for (i = 1; i <= n; i++) {  /* write the result back */
      lua_rawgeti(L, src, i);
      lua_seti(L, 1, i);
    }
  }
  return 0;
}

/* }====================================================== */


static const luaL_Reg tab_funcs[] = {
  {"concat", tconcat},
#if defined(LUA_COMPAT_MAXN)
  {"maxn", maxn},
#endif
  {"insert", tinsert},
  {"pack", pack},
  {"unpack", unpack},
  {"remove", tremove},
  {"move", tmove},
  {"sort", sort},
  {NULL, NULL}
};


LUAMOD_API int luaopen_table (lua_State *L) {
  luaL_newlib(L, tab_funcs);
#if defined(LUA_COMPAT_UNPACK)
  /* _G.unpack = table.unpack */
  lua_getfield(L, -1, "unpack");
  lua_setglobal(L, "unpack");
#endif
  return 1;
}

