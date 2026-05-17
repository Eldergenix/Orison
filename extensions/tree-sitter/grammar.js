/**
 * TreeSitter grammar for the Orison programming language.
 *
 * Covers the full bootstrap-parseable subset:
 * - module decl, import
 * - type decl (record / variant / newtype `wraps`)
 * - fn, service, view, actor, query, migration, capability, protocol, impl
 * - body expressions: literal, var, call, field/index access, if/else, match,
 *   return, throw, await, try (`?`), construct (record literal), list literal
 * - operators with precedence table matching crates/ori-compiler/src/expr_ops.rs
 *   precedence levels 1..7
 *
 * Precedence table (higher = tighter):
 *   1  ||           (left)
 *   2  &&           (left)
 *   3  == !=        (left)
 *   4  < <= > >=    (left)
 *   5  + -          (left)
 *   6  * / %        (left)
 *   7  ??           (right)
 *
 * Conflicts:
 * - record-literal `Foo { ... }` vs. block-bodied call: resolved by requiring
 *   a TitleCase identifier before `{` for the construct form; lowercase
 *   identifiers cannot construct, eliminating the ambiguity at scan time.
 * - `|` in variant declarations vs. binary OR (`||`): variant arms always sit
 *   at the start of a line and use single `|`, never `||`, so the lexer's
 *   greedy match disambiguates.
 * - prefix `-` vs. infix `-`: handled by the Pratt-style `prec` calls below;
 *   unary lives at PREC.UNARY which is strictly higher than any binary level.
 */

const PREC = {
  // Body-expression operator ladder, mirrors expr_ops.rs.
  OR: 1,
  AND: 2,
  EQUALITY: 3,
  COMPARISON: 4,
  ADDITIVE: 5,
  MULTIPLICATIVE: 6,
  COALESCE: 7,
  // Above the binary ladder.
  UNARY: 8,
  CALL: 9,
  FIELD: 10,
  TRY: 11,
};

module.exports = grammar({
  name: 'orison',

  extras: $ => [
    /\s+/,
    $.line_comment,
    $.block_comment,
  ],

  word: $ => $.identifier,

  conflicts: $ => [
    // `Foo` could be a type reference or the head of a record literal; the
    // parser needs one-token lookahead at the `{` to choose. We keep both
    // alternatives and let GLR resolve.
    [$.type_ref, $.construct_expr],
  ],

  rules: {
    source_file: $ => seq(
      optional($.module_decl),
      repeat($._top_item),
    ),

    // ---- declarations ---------------------------------------------------

    module_decl: $ => seq(
      'module',
      field('path', $.dotted_path),
    ),

    _top_item: $ => choice(
      $.import_decl,
      $.type_decl,
      $.fn_decl,
      $.service_decl,
      $.view_decl,
      $.actor_decl,
      $.query_decl,
      $.migration_decl,
      $.capability_decl,
      $.protocol_decl,
      $.impl_decl,
    ),

    import_decl: $ => seq(
      'import',
      field('path', $.dotted_path),
    ),

    dotted_path: $ => seq(
      $.identifier,
      repeat(seq('.', $.identifier)),
    ),

    type_decl: $ => seq(
      'type',
      field('name', $.type_identifier),
      choice(
        $.newtype_body,
        seq('=', $._type_body),
        $._record_or_variant_inline,
      ),
    ),

    newtype_body: $ => seq('wraps', field('inner', $._type)),

    _record_or_variant_inline: $ => choice(
      $.record_type,
      $.variant_type,
    ),

    _type_body: $ => choice(
      $.record_type,
      $.variant_type,
      $._type,
    ),

    record_type: $ => seq(
      '{',
      optional(seq(
        $.record_field,
        repeat(seq(',', $.record_field)),
        optional(','),
      )),
      '}',
    ),

    record_field: $ => seq(
      field('name', $.identifier),
      ':',
      field('type', $._type),
    ),

    variant_type: $ => repeat1($.variant_arm),

    variant_arm: $ => seq(
      '|',
      field('name', $.type_identifier),
      optional(seq(
        '(',
        optional(seq(
          $.variant_payload_field,
          repeat(seq(',', $.variant_payload_field)),
          optional(','),
        )),
        ')',
      )),
    ),

    variant_payload_field: $ => choice(
      seq(field('name', $.identifier), ':', field('type', $._type)),
      field('type', $._type),
    ),

    fn_decl: $ => seq(
      repeat(choice('async', 'extern', 'unsafe')),
      'fn',
      field('name', $.identifier),
      $.param_list,
      optional(seq('->', field('return_type', $._type))),
      optional($.uses_clause),
      ':',
      field('body', $.block),
    ),

    service_decl: $ => seq(
      'service',
      field('name', $.type_identifier),
      optional($.uses_clause),
      optional(seq(':', field('body', $.block))),
    ),

    view_decl: $ => seq(
      'view',
      field('name', $.type_identifier),
      $.param_list,
      optional(seq('->', field('return_type', $._type))),
      optional($.uses_clause),
      ':',
      field('body', $.block),
    ),

    actor_decl: $ => seq(
      'actor',
      field('name', $.type_identifier),
      optional($.uses_clause),
      optional(seq(':', field('body', $.block))),
    ),

    query_decl: $ => seq(
      'query',
      field('name', $.identifier),
      optional($.param_list),
      optional(seq('->', field('return_type', $._type))),
      optional($.uses_clause),
      ':',
      field('body', $.block),
    ),

    migration_decl: $ => seq(
      'migration',
      field('name', $.identifier),
      ':',
      repeat1(choice(
        seq('up', $.string_literal),
        seq('down', $.string_literal),
      )),
    ),

    capability_decl: $ => seq(
      'capability',
      field('name', $.dotted_path),
    ),

    protocol_decl: $ => seq(
      'protocol',
      field('name', $.type_identifier),
      ':',
      field('body', $.block),
    ),

    impl_decl: $ => seq(
      'impl',
      field('name', $.type_identifier),
      ':',
      field('body', $.block),
    ),

    param_list: $ => seq(
      '(',
      optional(seq(
        $.param,
        repeat(seq(',', $.param)),
        optional(','),
      )),
      ')',
    ),

    param: $ => seq(
      field('name', $.identifier),
      ':',
      field('type', $._type),
    ),

    uses_clause: $ => seq(
      'uses',
      $.capability_ref,
      repeat(seq(',', $.capability_ref)),
    ),

    capability_ref: $ => $.dotted_path,

    // ---- types ----------------------------------------------------------

    _type: $ => choice(
      $.generic_type,
      $.type_ref,
      $.record_type,
    ),

    type_ref: $ => $.type_identifier,

    generic_type: $ => seq(
      field('name', $.type_identifier),
      '[',
      $._type,
      repeat(seq(',', $._type)),
      ']',
    ),

    // ---- expressions / body --------------------------------------------

    block: $ => choice(
      $._expr_or_stmt,
      // Allow multiple statements separated by newlines/semicolons. The
      // surface syntax is indentation-led; the LSP/parser provide layout
      // tokens for the real compiler. For TreeSitter we accept any sequence.
      repeat1($._expr_or_stmt),
    ),

    _expr_or_stmt: $ => choice(
      $.let_stmt,
      $.return_stmt,
      $.throw_stmt,
      $.if_expr,
      $.match_expr,
      $.for_expr,
      $.while_expr,
      $._expression,
    ),

    let_stmt: $ => seq(
      choice('let', 'var'),
      optional('mut'),
      field('name', $.identifier),
      optional(seq(':', field('type', $._type))),
      '=',
      field('value', $._expression),
    ),

    return_stmt: $ => seq(
      'return',
      optional(field('value', $._expression)),
    ),

    throw_stmt: $ => seq('throw', field('value', $._expression)),

    if_expr: $ => prec.right(seq(
      'if',
      field('cond', $._expression),
      ':',
      field('then', $.block),
      optional(seq('else', ':', field('else', $.block))),
    )),

    match_expr: $ => seq(
      'match',
      field('subject', $._expression),
      ':',
      repeat1($.match_arm),
    ),

    match_arm: $ => seq(
      '|',
      field('pattern', $._pattern),
      '=>',
      field('body', $._expression),
    ),

    _pattern: $ => choice(
      $.variant_pattern,
      $.identifier,
      $.literal_expr,
    ),

    variant_pattern: $ => seq(
      $.type_identifier,
      optional(seq(
        '(',
        optional(seq(
          $._pattern,
          repeat(seq(',', $._pattern)),
        )),
        ')',
      )),
    ),

    for_expr: $ => seq(
      'for',
      field('binding', $.identifier),
      'in',
      field('iter', $._expression),
      ':',
      field('body', $.block),
    ),

    while_expr: $ => seq(
      'while',
      field('cond', $._expression),
      ':',
      field('body', $.block),
    ),

    _expression: $ => choice(
      $.binary_expr,
      $.unary_expr,
      $.try_expr,
      $.call_expr,
      $.field_access,
      $.index_access,
      $.construct_expr,
      $.list_literal,
      $.literal_expr,
      $.parenthesized_expr,
      $.identifier,
      $.await_expr,
    ),

    parenthesized_expr: $ => seq('(', $._expression, ')'),

    await_expr: $ => prec(PREC.UNARY, seq('await', $._expression)),

    binary_expr: $ => {
      const table = [
        [PREC.OR, '||', 'left'],
        [PREC.AND, '&&', 'left'],
        [PREC.EQUALITY, '==', 'left'],
        [PREC.EQUALITY, '!=', 'left'],
        [PREC.COMPARISON, '<', 'left'],
        [PREC.COMPARISON, '<=', 'left'],
        [PREC.COMPARISON, '>', 'left'],
        [PREC.COMPARISON, '>=', 'left'],
        [PREC.ADDITIVE, '+', 'left'],
        [PREC.ADDITIVE, '-', 'left'],
        [PREC.MULTIPLICATIVE, '*', 'left'],
        [PREC.MULTIPLICATIVE, '/', 'left'],
        [PREC.MULTIPLICATIVE, '%', 'left'],
        [PREC.COALESCE, '??', 'right'],
      ];
      return choice(...table.map(([precedence, operator, assoc]) => {
        const builder = assoc === 'right' ? prec.right : prec.left;
        return builder(precedence, seq(
          field('left', $._expression),
          field('op', operator),
          field('right', $._expression),
        ));
      }));
    },

    unary_expr: $ => prec(PREC.UNARY, seq(
      field('op', choice('-', '!')),
      field('operand', $._expression),
    )),

    try_expr: $ => prec(PREC.TRY, seq(
      field('value', $._expression),
      '?',
    )),

    call_expr: $ => prec(PREC.CALL, seq(
      field('callee', $._expression),
      '(',
      optional(seq(
        $._call_argument,
        repeat(seq(',', $._call_argument)),
        optional(','),
      )),
      ')',
    )),

    _call_argument: $ => choice(
      $.named_argument,
      $._expression,
    ),

    named_argument: $ => seq(
      field('name', $.identifier),
      ':',
      field('value', $._expression),
    ),

    field_access: $ => prec(PREC.FIELD, seq(
      field('object', $._expression),
      '.',
      field('field', $.identifier),
    )),

    index_access: $ => prec(PREC.FIELD, seq(
      field('object', $._expression),
      '[',
      field('index', $._expression),
      ']',
    )),

    construct_expr: $ => prec(PREC.CALL, seq(
      field('type', $.type_identifier),
      '{',
      optional(seq(
        $.construct_field,
        repeat(seq(',', $.construct_field)),
        optional(','),
      )),
      '}',
    )),

    construct_field: $ => seq(
      field('name', $.identifier),
      ':',
      field('value', $._expression),
    ),

    list_literal: $ => seq(
      '[',
      optional(seq(
        $._expression,
        repeat(seq(',', $._expression)),
        optional(','),
      )),
      ']',
    ),

    literal_expr: $ => choice(
      $.string_literal,
      $.number_literal,
      $.bool_literal,
      $.unit_literal,
    ),

    bool_literal: $ => choice('true', 'false'),

    unit_literal: $ => 'Unit',

    string_literal: $ => seq(
      '"',
      repeat(choice(
        $.string_content,
        $.escape_sequence,
        $.string_interpolation,
      )),
      '"',
    ),

    string_content: $ => token.immediate(prec(1, /[^"\\{]+/)),

    escape_sequence: $ => token.immediate(/\\([nrtv0\\"']|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\})/),

    string_interpolation: $ => seq(
      '{',
      $._expression,
      '}',
    ),

    number_literal: $ => choice(
      // Hex / binary / octal must precede decimal so the `0x`/`0b`/`0o`
      // prefix wins over the leading-zero decimal rule.
      /0x[0-9a-fA-F][0-9a-fA-F_]*/,
      /0b[01][01_]*/,
      /0o[0-7][0-7_]*/,
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?/,
      /[0-9][0-9_]*/,
    ),

    // ---- lexical primitives --------------------------------------------

    identifier: $ => /[a-z_][A-Za-z0-9_]*/,
    type_identifier: $ => /[A-Z][A-Za-z0-9_]*/,

    line_comment: $ => token(seq('//', /[^\n]*/)),
    block_comment: $ => token(seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/')),
  },
});
