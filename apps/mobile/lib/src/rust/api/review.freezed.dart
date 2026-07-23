// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'review.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$NoteUnit {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is NoteUnit);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'NoteUnit()';
}


}

/// @nodoc
class $NoteUnitCopyWith<$Res>  {
$NoteUnitCopyWith(NoteUnit _, $Res Function(NoteUnit) __);
}


/// Adds pattern-matching-related methods to [NoteUnit].
extension NoteUnitPatterns on NoteUnit {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( NoteUnit_Sentence value)?  sentence,TResult Function( NoteUnit_Code value)?  code,TResult Function( NoteUnit_Checklist value)?  checklist,required TResult orElse(),}){
final _that = this;
switch (_that) {
case NoteUnit_Sentence() when sentence != null:
return sentence(_that);case NoteUnit_Code() when code != null:
return code(_that);case NoteUnit_Checklist() when checklist != null:
return checklist(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( NoteUnit_Sentence value)  sentence,required TResult Function( NoteUnit_Code value)  code,required TResult Function( NoteUnit_Checklist value)  checklist,}){
final _that = this;
switch (_that) {
case NoteUnit_Sentence():
return sentence(_that);case NoteUnit_Code():
return code(_that);case NoteUnit_Checklist():
return checklist(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( NoteUnit_Sentence value)?  sentence,TResult? Function( NoteUnit_Code value)?  code,TResult? Function( NoteUnit_Checklist value)?  checklist,}){
final _that = this;
switch (_that) {
case NoteUnit_Sentence() when sentence != null:
return sentence(_that);case NoteUnit_Code() when code != null:
return code(_that);case NoteUnit_Checklist() when checklist != null:
return checklist(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String text,  List<InlineRun> runs)?  sentence,TResult Function( List<String> lines)?  code,TResult Function( List<ChecklistItem> items)?  checklist,required TResult orElse(),}) {final _that = this;
switch (_that) {
case NoteUnit_Sentence() when sentence != null:
return sentence(_that.text,_that.runs);case NoteUnit_Code() when code != null:
return code(_that.lines);case NoteUnit_Checklist() when checklist != null:
return checklist(_that.items);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String text,  List<InlineRun> runs)  sentence,required TResult Function( List<String> lines)  code,required TResult Function( List<ChecklistItem> items)  checklist,}) {final _that = this;
switch (_that) {
case NoteUnit_Sentence():
return sentence(_that.text,_that.runs);case NoteUnit_Code():
return code(_that.lines);case NoteUnit_Checklist():
return checklist(_that.items);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String text,  List<InlineRun> runs)?  sentence,TResult? Function( List<String> lines)?  code,TResult? Function( List<ChecklistItem> items)?  checklist,}) {final _that = this;
switch (_that) {
case NoteUnit_Sentence() when sentence != null:
return sentence(_that.text,_that.runs);case NoteUnit_Code() when code != null:
return code(_that.lines);case NoteUnit_Checklist() when checklist != null:
return checklist(_that.items);case _:
  return null;

}
}

}

/// @nodoc


class NoteUnit_Sentence extends NoteUnit {
  const NoteUnit_Sentence({required this.text, required final  List<InlineRun> runs}): _runs = runs,super._();
  

 final  String text;
 final  List<InlineRun> _runs;
 List<InlineRun> get runs {
  if (_runs is EqualUnmodifiableListView) return _runs;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_runs);
}


/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoteUnit_SentenceCopyWith<NoteUnit_Sentence> get copyWith => _$NoteUnit_SentenceCopyWithImpl<NoteUnit_Sentence>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is NoteUnit_Sentence&&(identical(other.text, text) || other.text == text)&&const DeepCollectionEquality().equals(other._runs, _runs));
}


@override
int get hashCode => Object.hash(runtimeType,text,const DeepCollectionEquality().hash(_runs));

@override
String toString() {
  return 'NoteUnit.sentence(text: $text, runs: $runs)';
}


}

/// @nodoc
abstract mixin class $NoteUnit_SentenceCopyWith<$Res> implements $NoteUnitCopyWith<$Res> {
  factory $NoteUnit_SentenceCopyWith(NoteUnit_Sentence value, $Res Function(NoteUnit_Sentence) _then) = _$NoteUnit_SentenceCopyWithImpl;
@useResult
$Res call({
 String text, List<InlineRun> runs
});




}
/// @nodoc
class _$NoteUnit_SentenceCopyWithImpl<$Res>
    implements $NoteUnit_SentenceCopyWith<$Res> {
  _$NoteUnit_SentenceCopyWithImpl(this._self, this._then);

  final NoteUnit_Sentence _self;
  final $Res Function(NoteUnit_Sentence) _then;

/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? text = null,Object? runs = null,}) {
  return _then(NoteUnit_Sentence(
text: null == text ? _self.text : text // ignore: cast_nullable_to_non_nullable
as String,runs: null == runs ? _self._runs : runs // ignore: cast_nullable_to_non_nullable
as List<InlineRun>,
  ));
}


}

/// @nodoc


class NoteUnit_Code extends NoteUnit {
  const NoteUnit_Code({required final  List<String> lines}): _lines = lines,super._();
  

 final  List<String> _lines;
 List<String> get lines {
  if (_lines is EqualUnmodifiableListView) return _lines;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_lines);
}


/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoteUnit_CodeCopyWith<NoteUnit_Code> get copyWith => _$NoteUnit_CodeCopyWithImpl<NoteUnit_Code>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is NoteUnit_Code&&const DeepCollectionEquality().equals(other._lines, _lines));
}


@override
int get hashCode => Object.hash(runtimeType,const DeepCollectionEquality().hash(_lines));

@override
String toString() {
  return 'NoteUnit.code(lines: $lines)';
}


}

/// @nodoc
abstract mixin class $NoteUnit_CodeCopyWith<$Res> implements $NoteUnitCopyWith<$Res> {
  factory $NoteUnit_CodeCopyWith(NoteUnit_Code value, $Res Function(NoteUnit_Code) _then) = _$NoteUnit_CodeCopyWithImpl;
@useResult
$Res call({
 List<String> lines
});




}
/// @nodoc
class _$NoteUnit_CodeCopyWithImpl<$Res>
    implements $NoteUnit_CodeCopyWith<$Res> {
  _$NoteUnit_CodeCopyWithImpl(this._self, this._then);

  final NoteUnit_Code _self;
  final $Res Function(NoteUnit_Code) _then;

/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? lines = null,}) {
  return _then(NoteUnit_Code(
lines: null == lines ? _self._lines : lines // ignore: cast_nullable_to_non_nullable
as List<String>,
  ));
}


}

/// @nodoc


class NoteUnit_Checklist extends NoteUnit {
  const NoteUnit_Checklist({required final  List<ChecklistItem> items}): _items = items,super._();


 final  List<ChecklistItem> _items;
 List<ChecklistItem> get items {
  if (_items is EqualUnmodifiableListView) return _items;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_items);
}


/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoteUnit_ChecklistCopyWith<NoteUnit_Checklist> get copyWith => _$NoteUnit_ChecklistCopyWithImpl<NoteUnit_Checklist>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is NoteUnit_Checklist&&const DeepCollectionEquality().equals(other._items, _items));
}


@override
int get hashCode => Object.hash(runtimeType,const DeepCollectionEquality().hash(_items));

@override
String toString() {
  return 'NoteUnit.checklist(items: $items)';
}


}

/// @nodoc
abstract mixin class $NoteUnit_ChecklistCopyWith<$Res> implements $NoteUnitCopyWith<$Res> {
  factory $NoteUnit_ChecklistCopyWith(NoteUnit_Checklist value, $Res Function(NoteUnit_Checklist) _then) = _$NoteUnit_ChecklistCopyWithImpl;
@useResult
$Res call({
 List<ChecklistItem> items
});




}
/// @nodoc
class _$NoteUnit_ChecklistCopyWithImpl<$Res>
    implements $NoteUnit_ChecklistCopyWith<$Res> {
  _$NoteUnit_ChecklistCopyWithImpl(this._self, this._then);

  final NoteUnit_Checklist _self;
  final $Res Function(NoteUnit_Checklist) _then;

/// Create a copy of NoteUnit
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? items = null,}) {
  return _then(NoteUnit_Checklist(
items: null == items ? _self._items : items // ignore: cast_nullable_to_non_nullable
as List<ChecklistItem>,
  ));
}


}

// dart format on
