// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'sync.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$PairedConflict {

 BigInt? get pulledRevision;
/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PairedConflictCopyWith<PairedConflict> get copyWith => _$PairedConflictCopyWithImpl<PairedConflict>(this as PairedConflict, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PairedConflict&&(identical(other.pulledRevision, pulledRevision) || other.pulledRevision == pulledRevision));
}


@override
int get hashCode => Object.hash(runtimeType,pulledRevision);

@override
String toString() {
  return 'PairedConflict(pulledRevision: $pulledRevision)';
}


}

/// @nodoc
abstract mixin class $PairedConflictCopyWith<$Res>  {
  factory $PairedConflictCopyWith(PairedConflict value, $Res Function(PairedConflict) _then) = _$PairedConflictCopyWithImpl;
@useResult
$Res call({
 BigInt? pulledRevision
});




}
/// @nodoc
class _$PairedConflictCopyWithImpl<$Res>
    implements $PairedConflictCopyWith<$Res> {
  _$PairedConflictCopyWithImpl(this._self, this._then);

  final PairedConflict _self;
  final $Res Function(PairedConflict) _then;

/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? pulledRevision = freezed,}) {
  return _then(_self.copyWith(
pulledRevision: freezed == pulledRevision ? _self.pulledRevision : pulledRevision // ignore: cast_nullable_to_non_nullable
as BigInt?,
  ));
}

}


/// Adds pattern-matching-related methods to [PairedConflict].
extension PairedConflictPatterns on PairedConflict {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( PairedConflict_Push value)?  push,TResult Function( PairedConflict_Pull value)?  pull,required TResult orElse(),}){
final _that = this;
switch (_that) {
case PairedConflict_Push() when push != null:
return push(_that);case PairedConflict_Pull() when pull != null:
return pull(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( PairedConflict_Push value)  push,required TResult Function( PairedConflict_Pull value)  pull,}){
final _that = this;
switch (_that) {
case PairedConflict_Push():
return push(_that);case PairedConflict_Pull():
return pull(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( PairedConflict_Push value)?  push,TResult? Function( PairedConflict_Pull value)?  pull,}){
final _that = this;
switch (_that) {
case PairedConflict_Push() when push != null:
return push(_that);case PairedConflict_Pull() when pull != null:
return pull(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( BigInt? desktopRevision,  BigInt? pulledRevision,  PairedWriter? desktopWriter)?  push,TResult Function( BigInt? pulledRevision,  PairedWriter? pulledWriter)?  pull,required TResult orElse(),}) {final _that = this;
switch (_that) {
case PairedConflict_Push() when push != null:
return push(_that.desktopRevision,_that.pulledRevision,_that.desktopWriter);case PairedConflict_Pull() when pull != null:
return pull(_that.pulledRevision,_that.pulledWriter);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( BigInt? desktopRevision,  BigInt? pulledRevision,  PairedWriter? desktopWriter)  push,required TResult Function( BigInt? pulledRevision,  PairedWriter? pulledWriter)  pull,}) {final _that = this;
switch (_that) {
case PairedConflict_Push():
return push(_that.desktopRevision,_that.pulledRevision,_that.desktopWriter);case PairedConflict_Pull():
return pull(_that.pulledRevision,_that.pulledWriter);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( BigInt? desktopRevision,  BigInt? pulledRevision,  PairedWriter? desktopWriter)?  push,TResult? Function( BigInt? pulledRevision,  PairedWriter? pulledWriter)?  pull,}) {final _that = this;
switch (_that) {
case PairedConflict_Push() when push != null:
return push(_that.desktopRevision,_that.pulledRevision,_that.desktopWriter);case PairedConflict_Pull() when pull != null:
return pull(_that.pulledRevision,_that.pulledWriter);case _:
  return null;

}
}

}

/// @nodoc


class PairedConflict_Push extends PairedConflict {
  const PairedConflict_Push({this.desktopRevision, this.pulledRevision, this.desktopWriter}): super._();
  

 final  BigInt? desktopRevision;
@override final  BigInt? pulledRevision;
 final  PairedWriter? desktopWriter;

/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PairedConflict_PushCopyWith<PairedConflict_Push> get copyWith => _$PairedConflict_PushCopyWithImpl<PairedConflict_Push>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PairedConflict_Push&&(identical(other.desktopRevision, desktopRevision) || other.desktopRevision == desktopRevision)&&(identical(other.pulledRevision, pulledRevision) || other.pulledRevision == pulledRevision)&&(identical(other.desktopWriter, desktopWriter) || other.desktopWriter == desktopWriter));
}


@override
int get hashCode => Object.hash(runtimeType,desktopRevision,pulledRevision,desktopWriter);

@override
String toString() {
  return 'PairedConflict.push(desktopRevision: $desktopRevision, pulledRevision: $pulledRevision, desktopWriter: $desktopWriter)';
}


}

/// @nodoc
abstract mixin class $PairedConflict_PushCopyWith<$Res> implements $PairedConflictCopyWith<$Res> {
  factory $PairedConflict_PushCopyWith(PairedConflict_Push value, $Res Function(PairedConflict_Push) _then) = _$PairedConflict_PushCopyWithImpl;
@override @useResult
$Res call({
 BigInt? desktopRevision, BigInt? pulledRevision, PairedWriter? desktopWriter
});




}
/// @nodoc
class _$PairedConflict_PushCopyWithImpl<$Res>
    implements $PairedConflict_PushCopyWith<$Res> {
  _$PairedConflict_PushCopyWithImpl(this._self, this._then);

  final PairedConflict_Push _self;
  final $Res Function(PairedConflict_Push) _then;

/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? desktopRevision = freezed,Object? pulledRevision = freezed,Object? desktopWriter = freezed,}) {
  return _then(PairedConflict_Push(
desktopRevision: freezed == desktopRevision ? _self.desktopRevision : desktopRevision // ignore: cast_nullable_to_non_nullable
as BigInt?,pulledRevision: freezed == pulledRevision ? _self.pulledRevision : pulledRevision // ignore: cast_nullable_to_non_nullable
as BigInt?,desktopWriter: freezed == desktopWriter ? _self.desktopWriter : desktopWriter // ignore: cast_nullable_to_non_nullable
as PairedWriter?,
  ));
}


}

/// @nodoc


class PairedConflict_Pull extends PairedConflict {
  const PairedConflict_Pull({this.pulledRevision, this.pulledWriter}): super._();
  

@override final  BigInt? pulledRevision;
 final  PairedWriter? pulledWriter;

/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PairedConflict_PullCopyWith<PairedConflict_Pull> get copyWith => _$PairedConflict_PullCopyWithImpl<PairedConflict_Pull>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PairedConflict_Pull&&(identical(other.pulledRevision, pulledRevision) || other.pulledRevision == pulledRevision)&&(identical(other.pulledWriter, pulledWriter) || other.pulledWriter == pulledWriter));
}


@override
int get hashCode => Object.hash(runtimeType,pulledRevision,pulledWriter);

@override
String toString() {
  return 'PairedConflict.pull(pulledRevision: $pulledRevision, pulledWriter: $pulledWriter)';
}


}

/// @nodoc
abstract mixin class $PairedConflict_PullCopyWith<$Res> implements $PairedConflictCopyWith<$Res> {
  factory $PairedConflict_PullCopyWith(PairedConflict_Pull value, $Res Function(PairedConflict_Pull) _then) = _$PairedConflict_PullCopyWithImpl;
@override @useResult
$Res call({
 BigInt? pulledRevision, PairedWriter? pulledWriter
});




}
/// @nodoc
class _$PairedConflict_PullCopyWithImpl<$Res>
    implements $PairedConflict_PullCopyWith<$Res> {
  _$PairedConflict_PullCopyWithImpl(this._self, this._then);

  final PairedConflict_Pull _self;
  final $Res Function(PairedConflict_Pull) _then;

/// Create a copy of PairedConflict
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? pulledRevision = freezed,Object? pulledWriter = freezed,}) {
  return _then(PairedConflict_Pull(
pulledRevision: freezed == pulledRevision ? _self.pulledRevision : pulledRevision // ignore: cast_nullable_to_non_nullable
as BigInt?,pulledWriter: freezed == pulledWriter ? _self.pulledWriter : pulledWriter // ignore: cast_nullable_to_non_nullable
as PairedWriter?,
  ));
}


}

/// @nodoc
mixin _$PushOutcomeDto {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PushOutcomeDto);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'PushOutcomeDto()';
}


}

/// @nodoc
class $PushOutcomeDtoCopyWith<$Res>  {
$PushOutcomeDtoCopyWith(PushOutcomeDto _, $Res Function(PushOutcomeDto) __);
}


/// Adds pattern-matching-related methods to [PushOutcomeDto].
extension PushOutcomeDtoPatterns on PushOutcomeDto {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( PushOutcomeDto_Accepted value)?  accepted,TResult Function( PushOutcomeDto_Conflict value)?  conflict,TResult Function( PushOutcomeDto_NotServed value)?  notServed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted() when accepted != null:
return accepted(_that);case PushOutcomeDto_Conflict() when conflict != null:
return conflict(_that);case PushOutcomeDto_NotServed() when notServed != null:
return notServed(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( PushOutcomeDto_Accepted value)  accepted,required TResult Function( PushOutcomeDto_Conflict value)  conflict,required TResult Function( PushOutcomeDto_NotServed value)  notServed,}){
final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted():
return accepted(_that);case PushOutcomeDto_Conflict():
return conflict(_that);case PushOutcomeDto_NotServed():
return notServed(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( PushOutcomeDto_Accepted value)?  accepted,TResult? Function( PushOutcomeDto_Conflict value)?  conflict,TResult? Function( PushOutcomeDto_NotServed value)?  notServed,}){
final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted() when accepted != null:
return accepted(_that);case PushOutcomeDto_Conflict() when conflict != null:
return conflict(_that);case PushOutcomeDto_NotServed() when notServed != null:
return notServed(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( BigInt revision)?  accepted,TResult Function( BigInt? desktopRevision,  PairedWriter? desktopWriter)?  conflict,TResult Function()?  notServed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted() when accepted != null:
return accepted(_that.revision);case PushOutcomeDto_Conflict() when conflict != null:
return conflict(_that.desktopRevision,_that.desktopWriter);case PushOutcomeDto_NotServed() when notServed != null:
return notServed();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( BigInt revision)  accepted,required TResult Function( BigInt? desktopRevision,  PairedWriter? desktopWriter)  conflict,required TResult Function()  notServed,}) {final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted():
return accepted(_that.revision);case PushOutcomeDto_Conflict():
return conflict(_that.desktopRevision,_that.desktopWriter);case PushOutcomeDto_NotServed():
return notServed();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( BigInt revision)?  accepted,TResult? Function( BigInt? desktopRevision,  PairedWriter? desktopWriter)?  conflict,TResult? Function()?  notServed,}) {final _that = this;
switch (_that) {
case PushOutcomeDto_Accepted() when accepted != null:
return accepted(_that.revision);case PushOutcomeDto_Conflict() when conflict != null:
return conflict(_that.desktopRevision,_that.desktopWriter);case PushOutcomeDto_NotServed() when notServed != null:
return notServed();case _:
  return null;

}
}

}

/// @nodoc


class PushOutcomeDto_Accepted extends PushOutcomeDto {
  const PushOutcomeDto_Accepted({required this.revision}): super._();
  

 final  BigInt revision;

/// Create a copy of PushOutcomeDto
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PushOutcomeDto_AcceptedCopyWith<PushOutcomeDto_Accepted> get copyWith => _$PushOutcomeDto_AcceptedCopyWithImpl<PushOutcomeDto_Accepted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PushOutcomeDto_Accepted&&(identical(other.revision, revision) || other.revision == revision));
}


@override
int get hashCode => Object.hash(runtimeType,revision);

@override
String toString() {
  return 'PushOutcomeDto.accepted(revision: $revision)';
}


}

/// @nodoc
abstract mixin class $PushOutcomeDto_AcceptedCopyWith<$Res> implements $PushOutcomeDtoCopyWith<$Res> {
  factory $PushOutcomeDto_AcceptedCopyWith(PushOutcomeDto_Accepted value, $Res Function(PushOutcomeDto_Accepted) _then) = _$PushOutcomeDto_AcceptedCopyWithImpl;
@useResult
$Res call({
 BigInt revision
});




}
/// @nodoc
class _$PushOutcomeDto_AcceptedCopyWithImpl<$Res>
    implements $PushOutcomeDto_AcceptedCopyWith<$Res> {
  _$PushOutcomeDto_AcceptedCopyWithImpl(this._self, this._then);

  final PushOutcomeDto_Accepted _self;
  final $Res Function(PushOutcomeDto_Accepted) _then;

/// Create a copy of PushOutcomeDto
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? revision = null,}) {
  return _then(PushOutcomeDto_Accepted(
revision: null == revision ? _self.revision : revision // ignore: cast_nullable_to_non_nullable
as BigInt,
  ));
}


}

/// @nodoc


class PushOutcomeDto_Conflict extends PushOutcomeDto {
  const PushOutcomeDto_Conflict({this.desktopRevision, this.desktopWriter}): super._();
  

 final  BigInt? desktopRevision;
 final  PairedWriter? desktopWriter;

/// Create a copy of PushOutcomeDto
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PushOutcomeDto_ConflictCopyWith<PushOutcomeDto_Conflict> get copyWith => _$PushOutcomeDto_ConflictCopyWithImpl<PushOutcomeDto_Conflict>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PushOutcomeDto_Conflict&&(identical(other.desktopRevision, desktopRevision) || other.desktopRevision == desktopRevision)&&(identical(other.desktopWriter, desktopWriter) || other.desktopWriter == desktopWriter));
}


@override
int get hashCode => Object.hash(runtimeType,desktopRevision,desktopWriter);

@override
String toString() {
  return 'PushOutcomeDto.conflict(desktopRevision: $desktopRevision, desktopWriter: $desktopWriter)';
}


}

/// @nodoc
abstract mixin class $PushOutcomeDto_ConflictCopyWith<$Res> implements $PushOutcomeDtoCopyWith<$Res> {
  factory $PushOutcomeDto_ConflictCopyWith(PushOutcomeDto_Conflict value, $Res Function(PushOutcomeDto_Conflict) _then) = _$PushOutcomeDto_ConflictCopyWithImpl;
@useResult
$Res call({
 BigInt? desktopRevision, PairedWriter? desktopWriter
});




}
/// @nodoc
class _$PushOutcomeDto_ConflictCopyWithImpl<$Res>
    implements $PushOutcomeDto_ConflictCopyWith<$Res> {
  _$PushOutcomeDto_ConflictCopyWithImpl(this._self, this._then);

  final PushOutcomeDto_Conflict _self;
  final $Res Function(PushOutcomeDto_Conflict) _then;

/// Create a copy of PushOutcomeDto
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? desktopRevision = freezed,Object? desktopWriter = freezed,}) {
  return _then(PushOutcomeDto_Conflict(
desktopRevision: freezed == desktopRevision ? _self.desktopRevision : desktopRevision // ignore: cast_nullable_to_non_nullable
as BigInt?,desktopWriter: freezed == desktopWriter ? _self.desktopWriter : desktopWriter // ignore: cast_nullable_to_non_nullable
as PairedWriter?,
  ));
}


}

/// @nodoc


class PushOutcomeDto_NotServed extends PushOutcomeDto {
  const PushOutcomeDto_NotServed(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is PushOutcomeDto_NotServed);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'PushOutcomeDto.notServed()';
}


}




/// @nodoc
mixin _$ResolutionDto {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ResolutionDto);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ResolutionDto()';
}


}

/// @nodoc
class $ResolutionDtoCopyWith<$Res>  {
$ResolutionDtoCopyWith(ResolutionDto _, $Res Function(ResolutionDto) __);
}


/// Adds pattern-matching-related methods to [ResolutionDto].
extension ResolutionDtoPatterns on ResolutionDto {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ResolutionDto_Done value)?  done,TResult Function( ResolutionDto_Push value)?  push,TResult Function( ResolutionDto_Pull value)?  pull,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ResolutionDto_Done() when done != null:
return done(_that);case ResolutionDto_Push() when push != null:
return push(_that);case ResolutionDto_Pull() when pull != null:
return pull(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ResolutionDto_Done value)  done,required TResult Function( ResolutionDto_Push value)  push,required TResult Function( ResolutionDto_Pull value)  pull,}){
final _that = this;
switch (_that) {
case ResolutionDto_Done():
return done(_that);case ResolutionDto_Push():
return push(_that);case ResolutionDto_Pull():
return pull(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ResolutionDto_Done value)?  done,TResult? Function( ResolutionDto_Push value)?  push,TResult? Function( ResolutionDto_Pull value)?  pull,}){
final _that = this;
switch (_that) {
case ResolutionDto_Done() when done != null:
return done(_that);case ResolutionDto_Push() when push != null:
return push(_that);case ResolutionDto_Pull() when pull != null:
return pull(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  done,TResult Function( PushPlanItem item)?  push,TResult Function( String entry)?  pull,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ResolutionDto_Done() when done != null:
return done();case ResolutionDto_Push() when push != null:
return push(_that.item);case ResolutionDto_Pull() when pull != null:
return pull(_that.entry);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  done,required TResult Function( PushPlanItem item)  push,required TResult Function( String entry)  pull,}) {final _that = this;
switch (_that) {
case ResolutionDto_Done():
return done();case ResolutionDto_Push():
return push(_that.item);case ResolutionDto_Pull():
return pull(_that.entry);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  done,TResult? Function( PushPlanItem item)?  push,TResult? Function( String entry)?  pull,}) {final _that = this;
switch (_that) {
case ResolutionDto_Done() when done != null:
return done();case ResolutionDto_Push() when push != null:
return push(_that.item);case ResolutionDto_Pull() when pull != null:
return pull(_that.entry);case _:
  return null;

}
}

}

/// @nodoc


class ResolutionDto_Done extends ResolutionDto {
  const ResolutionDto_Done(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ResolutionDto_Done);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ResolutionDto.done()';
}


}




/// @nodoc


class ResolutionDto_Push extends ResolutionDto {
  const ResolutionDto_Push({required this.item}): super._();
  

 final  PushPlanItem item;

/// Create a copy of ResolutionDto
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ResolutionDto_PushCopyWith<ResolutionDto_Push> get copyWith => _$ResolutionDto_PushCopyWithImpl<ResolutionDto_Push>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ResolutionDto_Push&&(identical(other.item, item) || other.item == item));
}


@override
int get hashCode => Object.hash(runtimeType,item);

@override
String toString() {
  return 'ResolutionDto.push(item: $item)';
}


}

/// @nodoc
abstract mixin class $ResolutionDto_PushCopyWith<$Res> implements $ResolutionDtoCopyWith<$Res> {
  factory $ResolutionDto_PushCopyWith(ResolutionDto_Push value, $Res Function(ResolutionDto_Push) _then) = _$ResolutionDto_PushCopyWithImpl;
@useResult
$Res call({
 PushPlanItem item
});




}
/// @nodoc
class _$ResolutionDto_PushCopyWithImpl<$Res>
    implements $ResolutionDto_PushCopyWith<$Res> {
  _$ResolutionDto_PushCopyWithImpl(this._self, this._then);

  final ResolutionDto_Push _self;
  final $Res Function(ResolutionDto_Push) _then;

/// Create a copy of ResolutionDto
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? item = null,}) {
  return _then(ResolutionDto_Push(
item: null == item ? _self.item : item // ignore: cast_nullable_to_non_nullable
as PushPlanItem,
  ));
}


}

/// @nodoc


class ResolutionDto_Pull extends ResolutionDto {
  const ResolutionDto_Pull({required this.entry}): super._();
  

 final  String entry;

/// Create a copy of ResolutionDto
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ResolutionDto_PullCopyWith<ResolutionDto_Pull> get copyWith => _$ResolutionDto_PullCopyWithImpl<ResolutionDto_Pull>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ResolutionDto_Pull&&(identical(other.entry, entry) || other.entry == entry));
}


@override
int get hashCode => Object.hash(runtimeType,entry);

@override
String toString() {
  return 'ResolutionDto.pull(entry: $entry)';
}


}

/// @nodoc
abstract mixin class $ResolutionDto_PullCopyWith<$Res> implements $ResolutionDtoCopyWith<$Res> {
  factory $ResolutionDto_PullCopyWith(ResolutionDto_Pull value, $Res Function(ResolutionDto_Pull) _then) = _$ResolutionDto_PullCopyWithImpl;
@useResult
$Res call({
 String entry
});




}
/// @nodoc
class _$ResolutionDto_PullCopyWithImpl<$Res>
    implements $ResolutionDto_PullCopyWith<$Res> {
  _$ResolutionDto_PullCopyWithImpl(this._self, this._then);

  final ResolutionDto_Pull _self;
  final $Res Function(ResolutionDto_Pull) _then;

/// Create a copy of ResolutionDto
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? entry = null,}) {
  return _then(ResolutionDto_Pull(
entry: null == entry ? _self.entry : entry // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

// dart format on
